//! Machine-profile parsing and structural validation
//! (IO-Routing-and-Backends §2.2, §3).
//!
//! A profile is a TOML document with one `[device.<name>]` table per data
//! device and a top-level `[[routes]]` array. This module validates the
//! profile's shape: device addressing and PDO/sync tree types, unique entry
//! addresses, `pdo-channels` references, and route endpoint spelling and
//! source/sink pairing. It does not resolve routes against the program or the
//! device tree; that needs the checked HIR and happens during code generation.
//!
//! Profile errors use the project I/O code `STC-E-PROJECT-009`; route
//! direction/pairing and fan-in use `STC-E-IO-BINDING-002` and
//! `STC-E-IO-BINDING-001` (IO-Routing-and-Backends §10).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use stc_diag::{Diagnostic, Severity};
use stc_span::{FileId, Span};
use toml_edit::{Document, Item, TableLike, Value};

use crate::model::ConfigValue;
use crate::parse::parse_duration;

/// The maximum device-configuration nesting accepted from a profile.
const CONFIG_NESTING_LIMIT: usize = 64;

const PROFILE_KEYS: &[&str] = &["device", "routes"];
const DEVICE_KEYS: &[&str] = &[
    "master",
    "alias",
    "position",
    "vendor",
    "product",
    "revision",
    "startup-timeout",
    "recovery-cycles",
    "syncs",
    "pdo-channels",
    "dc",
];
const SYNC_KEYS: &[&str] = &["index", "direction", "watchdog", "pdos"];
const PDO_KEYS: &[&str] = &["index", "entries"];
const ENTRY_KEYS: &[&str] = &["index", "subindex", "bits"];
const PDO_CHANNEL_KEYS: &[&str] = &["name", "entry", "bit"];
const DC_KEYS: &[&str] = &[
    "assign-activate",
    "sync0-cycle",
    "sync0-shift",
    "sync1-cycle",
    "sync1-shift",
];
const ROUTE_KEYS: &[&str] = &["source", "sinks"];
const PHYSICAL_ENDPOINT_KEYS: &[&str] = &["kind", "device", "channel"];
const PLC_ENDPOINT_KEYS: &[&str] = &["kind", "variable"];

/// A parsed machine profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// `[device.<name>]` tables in declaration order.
    pub devices: Vec<ProfileDevice>,
    /// Top-level `[[routes]]` entries in declaration order.
    pub routes: Vec<ProfileRoute>,
}

/// One profile device table.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileDevice {
    /// Device name; must match a `[[io.devices]]` declaration.
    pub name: String,
    /// Validated device configuration as a TOML value tree.
    pub config: ConfigValue,
    /// Span of the `[device.<name>]` table.
    pub span: Span,
}

/// One `[[routes]]` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileRoute {
    /// The single source endpoint.
    pub source: ProfileEndpoint,
    /// The non-empty sink list.
    pub sinks: Vec<ProfileEndpoint>,
    /// Span of the route table.
    pub span: Span,
}

/// One route endpoint.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileEndpoint {
    /// A declared physical input channel.
    PhysicalInput {
        /// Device name.
        device: String,
        /// Channel name.
        channel: String,
        /// Span of the endpoint object.
        span: Span,
    },
    /// A declared physical output channel.
    PhysicalOutput {
        /// Device name.
        device: String,
        /// Channel name.
        channel: String,
        /// Span of the endpoint object.
        span: Span,
    },
    /// A direct `%I` variable.
    PlcInput {
        /// Variable path as written.
        variable: String,
        /// Span of the endpoint object.
        span: Span,
    },
    /// A direct `%Q` variable.
    PlcOutput {
        /// Variable path as written.
        variable: String,
        /// Span of the endpoint object.
        span: Span,
    },
}

impl ProfileEndpoint {
    /// Returns the fixed `kind` spelling.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::PhysicalInput { .. } => "physical-input",
            Self::PhysicalOutput { .. } => "physical-output",
            Self::PlcInput { .. } => "plc-input",
            Self::PlcOutput { .. } => "plc-output",
        }
    }

    /// Returns the endpoint span.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::PhysicalInput { span, .. }
            | Self::PhysicalOutput { span, .. }
            | Self::PlcInput { span, .. }
            | Self::PlcOutput { span, .. } => *span,
        }
    }

    /// Returns the physical device name, when this is a physical endpoint.
    #[must_use]
    pub fn device_name(&self) -> Option<&str> {
        match self {
            Self::PhysicalInput { device, .. } | Self::PhysicalOutput { device, .. } => {
                Some(device)
            }
            Self::PlcInput { .. } | Self::PlcOutput { .. } => None,
        }
    }

    /// Returns the physical channel name, when this is a physical endpoint.
    #[must_use]
    pub fn channel_name(&self) -> Option<&str> {
        match self {
            Self::PhysicalInput { channel, .. } | Self::PhysicalOutput { channel, .. } => {
                Some(channel)
            }
            Self::PlcInput { .. } | Self::PlcOutput { .. } => None,
        }
    }

    /// Returns the PLC variable path, when this is a PLC endpoint.
    #[must_use]
    pub fn variable_name(&self) -> Option<&str> {
        match self {
            Self::PlcInput { variable, .. } | Self::PlcOutput { variable, .. } => Some(variable),
            Self::PhysicalInput { .. } | Self::PhysicalOutput { .. } => None,
        }
    }
}

/// Parses and structurally validates one machine profile.
///
/// # Errors
///
/// Returns every diagnostic when at least one has error severity.
pub fn parse_profile(text: &str, file: FileId) -> Result<Profile, Vec<Diagnostic>> {
    let (stripped, base) = strip_bom(text);
    let mut parser = ProfileParser::new(file, base, text.len());
    let document = match Document::parse(stripped) {
        Ok(document) => document,
        Err(error) => {
            let span = parser.span(error.span());
            parser.error(
                "STC-E-PROJECT-001",
                format!("invalid I/O profile TOML: {}", error.message()),
                span,
            );
            return Err(parser.diagnostics);
        }
    };
    let profile = parser.parse_root(document.as_table());
    if parser.has_errors() {
        Err(parser.diagnostics)
    } else {
        Ok(profile)
    }
}

fn strip_bom(text: &str) -> (&str, usize) {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => (rest, 3),
        None => (text, 0),
    }
}

struct ProfileParser {
    file: FileId,
    base: usize,
    full_len: usize,
    diagnostics: Vec<Diagnostic>,
}

impl ProfileParser {
    fn new(file: FileId, base: usize, full_len: usize) -> Self {
        Self {
            file,
            base,
            full_len,
            diagnostics: Vec::new(),
        }
    }

    fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }

    fn span(&self, range: Option<Range<usize>>) -> Span {
        let range = range.unwrap_or(0..0);
        let start = self.base.saturating_add(range.start).min(self.full_len);
        let end = self
            .base
            .saturating_add(range.end)
            .max(start)
            .min(self.full_len);
        Span::new(
            self.file,
            u32::try_from(start).unwrap_or(u32::MAX),
            u32::try_from(end).unwrap_or(u32::MAX),
        )
    }

    fn item_span(&self, item: &Item) -> Span {
        self.span(item.span())
    }

    fn table_span(&self, table: &toml_edit::Table) -> Span {
        self.span(table.span())
    }

    fn value_span(&self, value: &Value) -> Span {
        self.span(value.span())
    }

    fn parse_root(&mut self, table: &dyn TableLike) -> Profile {
        let mut profile = Profile {
            devices: Vec::new(),
            routes: Vec::new(),
        };
        self.check_unknown(table, "the I/O profile", PROFILE_KEYS);
        if let Some(device) = table.get("device") {
            profile.devices = self.parse_devices(device);
        }
        if let Some(routes) = table.get("routes") {
            profile.routes = self.parse_routes(routes);
        }
        profile
    }

    fn check_unknown(&mut self, table: &dyn TableLike, owner: &str, keys: &[&str]) {
        for (key, _) in table.iter() {
            if !keys.contains(&key) {
                let span = table
                    .get_key_value(key)
                    .map_or_else(|| self.span(None), |(_, item)| self.item_span(item));
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "unknown key `{key}` in {owner}; expected one of {}",
                        render_keys(keys)
                    ),
                    span,
                );
            }
        }
    }

    fn parse_devices(&mut self, item: &Item) -> Vec<ProfileDevice> {
        let Some(table) = item.as_table_like() else {
            self.error(
                "STC-E-PROJECT-009",
                "`device` must be a table of `[device.<name>]` tables",
                self.item_span(item),
            );
            return Vec::new();
        };
        let mut devices = Vec::new();
        for (name, entry) in table.iter() {
            let Some(config) = entry.as_table_like() else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("profile device `{name}` must be a table"),
                    self.item_span(entry),
                );
                continue;
            };
            let span = self.item_span(entry);
            self.check_unknown(config, &format!("profile device `{name}`"), DEVICE_KEYS);
            self.validate_device(name, config);
            let value = self.config_table(config);
            devices.push(ProfileDevice {
                name: name.to_owned(),
                config: value,
                span,
            });
        }
        devices
    }

    fn validate_device(&mut self, device: &str, table: &dyn TableLike) {
        for key in ["master", "alias", "position"] {
            if let Some(item) = table.get(key)
                && let Some(value) = item.as_integer()
                && value < 0
            {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("profile device `{device}` `{key}` must be a non-negative integer"),
                    self.item_span(item),
                );
            }
        }
        for key in ["vendor", "product", "revision"] {
            if let Some(item) = table.get(key) {
                self.identity_value(item, device, key);
            }
        }
        if let Some(item) = table.get("startup-timeout") {
            match item.as_str().and_then(parse_duration) {
                Some(_) => {}
                None => self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` `startup-timeout` must be a duration such as `5s`"
                    ),
                    self.item_span(item),
                ),
            }
        }
        if let Some(item) = table.get("recovery-cycles")
            && let Some(value) = item.as_integer()
            && value < 0
        {
            self.error(
                "STC-E-PROJECT-009",
                format!(
                    "profile device `{device}` `recovery-cycles` must be a non-negative integer"
                ),
                self.item_span(item),
            );
        }
        if let Some(item) = table.get("syncs") {
            self.parse_syncs(device, item);
        }
        if let Some(item) = table.get("pdo-channels") {
            self.parse_pdo_channels(device, table, item);
        }
        if let Some(item) = table.get("dc") {
            self.parse_dc(device, item);
        }
    }

    fn identity_value(&mut self, item: &Item, device: &str, key: &str) {
        if let Some(value) = item.as_integer() {
            if value < 0 {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("profile device `{device}` `{key}` must be non-negative"),
                    self.item_span(item),
                );
            }
            return;
        }
        if let Some(text) = item.as_str() {
            if parse_identity(text).is_none() {
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` `{key}` must be an integer or a hexadecimal string"
                    ),
                    self.item_span(item),
                );
            }
            return;
        }
        self.error(
            "STC-E-PROJECT-009",
            format!("profile device `{device}` `{key}` must be an integer or a hexadecimal string"),
            self.item_span(item),
        );
    }

    fn parse_syncs(&mut self, device: &str, item: &Item) {
        let Some(syncs) = item.as_array_of_tables() else {
            self.error(
                "STC-E-PROJECT-009",
                format!("profile device `{device}` `syncs` must be an array of tables"),
                self.item_span(item),
            );
            return;
        };
        let mut entries: BTreeMap<(u32, u8), Span> = BTreeMap::new();
        for sync in syncs {
            self.check_unknown(sync, "sync", SYNC_KEYS);
            self.required_index(sync, "index", device, "sync");
            match sync.get("direction").and_then(Item::as_str) {
                Some("input" | "output") => {}
                _ => self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` sync `direction` must be `input` or `output`"
                    ),
                    sync.get("direction")
                        .map_or_else(|| self.span(None), |item| self.item_span(item)),
                ),
            }
            match sync.get("watchdog").and_then(Item::as_str) {
                Some("enable" | "disable") => {}
                _ => self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` sync `watchdog` must be `enable` or `disable`"
                    ),
                    sync.get("watchdog")
                        .map_or_else(|| self.span(None), |item| self.item_span(item)),
                ),
            }
            if let Some(pdos) = sync.get("pdos") {
                self.parse_pdos(device, pdos, &mut entries);
            }
        }
    }

    fn parse_pdos(&mut self, device: &str, item: &Item, entries: &mut BTreeMap<(u32, u8), Span>) {
        let Some(pdos) = item.as_array_of_tables() else {
            self.error(
                "STC-E-PROJECT-009",
                format!("profile device `{device}` `pdos` must be an array of tables"),
                self.item_span(item),
            );
            return;
        };
        for pdo in pdos {
            self.check_unknown(pdo, "pdo", PDO_KEYS);
            self.required_index(pdo, "index", device, "pdo");
            if let Some(item) = pdo.get("entries") {
                let Some(pdo_entries) = item.as_array_of_tables() else {
                    self.error(
                        "STC-E-PROJECT-009",
                        format!("profile device `{device}` `entries` must be an array of tables"),
                        self.item_span(item),
                    );
                    continue;
                };
                for entry in pdo_entries {
                    self.check_unknown(entry, "PDO entry", ENTRY_KEYS);
                    let Some(index) = self.required_index(entry, "index", device, "PDO entry")
                    else {
                        continue;
                    };
                    let Some(subindex) = entry.get("subindex").and_then(Item::as_integer) else {
                        self.error(
                            "STC-E-PROJECT-009",
                            format!(
                                "profile device `{device}` PDO entry `subindex` must be an integer"
                            ),
                            entry
                                .get("subindex")
                                .map_or_else(|| self.span(None), |item| self.item_span(item)),
                        );
                        continue;
                    };
                    if !(0..=255).contains(&subindex) {
                        self.error(
                            "STC-E-PROJECT-009",
                            format!(
                                "profile device `{device}` PDO entry `subindex` must be 0..=255"
                            ),
                            entry
                                .get("subindex")
                                .map_or_else(|| self.span(None), |item| self.item_span(item)),
                        );
                        continue;
                    }
                    let subindex = u8::try_from(subindex).unwrap_or(u8::MAX);
                    match entry.get("bits").and_then(Item::as_integer) {
                        Some(bits) if (1..=64).contains(&bits) => {}
                        _ => self.error(
                            "STC-E-PROJECT-009",
                            format!(
                                "profile device `{device}` PDO entry `bits` must be an integer in 1..=64"
                            ),
                            entry
                                .get("bits")
                                .map_or_else(|| self.span(None), |item| self.item_span(item)),
                        ),
                    }
                    let address = (index, subindex);
                    let span = self.table_span(entry);
                    if let Some(previous) = entries.insert(address, span) {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "STC-E-PROJECT-009",
                                format!(
                                    "profile device `{device}` declares PDO entry \
                                     `0x{index:04x}:{subindex}` more than once"
                                ),
                                span,
                            )
                            .with_label(previous, "first declared here"),
                        );
                    }
                }
            }
        }
    }

    fn required_index(
        &mut self,
        table: &dyn TableLike,
        key: &str,
        device: &str,
        owner: &str,
    ) -> Option<u32> {
        let Some(item) = table.get(key) else {
            self.error(
                "STC-E-PROJECT-009",
                format!("profile device `{device}` {owner} is missing `{key}`"),
                self.span(None),
            );
            return None;
        };
        if let Some(value) = index_value(item) {
            return Some(value);
        }
        self.error(
            "STC-E-PROJECT-009",
            format!(
                "profile device `{device}` {owner} `{key}` must be an integer or a hexadecimal string"
            ),
            self.item_span(item),
        );
        None
    }

    fn parse_pdo_channels(&mut self, device: &str, table: &dyn TableLike, item: &Item) {
        let Some(channels) = item.as_array_of_tables() else {
            self.error(
                "STC-E-PROJECT-009",
                format!("profile device `{device}` `pdo-channels` must be an array of tables"),
                self.item_span(item),
            );
            return;
        };
        let entries = declared_entries(table);
        let mut names = BTreeSet::new();
        for channel in channels {
            self.check_unknown(channel, "pdo-channel", PDO_CHANNEL_KEYS);
            let Some(name) = channel.get("name").and_then(Item::as_str) else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("profile device `{device}` `pdo-channels` entry is missing `name`"),
                    self.span(None),
                );
                continue;
            };
            if !names.insert(name.to_owned()) {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("profile device `{device}` maps channel `{name}` more than once"),
                    self.item_span(channel.get("name").unwrap_or(item)),
                );
            }
            let Some(entry) = channel.get("entry").and_then(Item::as_table_like) else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` channel `{name}` `entry` must be an inline table or table"
                    ),
                    channel
                        .get("entry")
                        .map_or_else(|| self.span(None), |item| self.item_span(item)),
                );
                continue;
            };
            let Some(index) = entry.get("index").and_then(index_value) else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` channel `{name}` entry `index` must be an integer or a hexadecimal string"
                    ),
                    self.item_span(entry.get("index").unwrap_or(item)),
                );
                continue;
            };
            let Some(subindex) = entry.get("subindex").and_then(Item::as_integer) else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` channel `{name}` entry `subindex` must be an integer"
                    ),
                    self.item_span(entry.get("subindex").unwrap_or(item)),
                );
                continue;
            };
            let Ok(subindex) = u8::try_from(subindex) else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` channel `{name}` entry `subindex` must be 0..=255"
                    ),
                    self.item_span(entry.get("subindex").unwrap_or(item)),
                );
                continue;
            };
            let Some(bits) = entries.get(&(index, subindex)) else {
                self.error(
                    "STC-E-PROJECT-009",
                    format!(
                        "profile device `{device}` channel `{name}` names entry `0x{index:04x}:{subindex}`, which is not declared"
                    ),
                    self.item_span(entry.get("index").unwrap_or(item)),
                );
                continue;
            };
            match channel.get("bit").and_then(Item::as_integer) {
                Some(bit) if bit >= 0 && bit < i64::from(*bits) => {}
                _ => {
                    self.error(
                        "STC-E-PROJECT-009",
                        format!(
                            "profile device `{device}` channel `{name}` `bit` must be inside the entry width of {bits} bits"
                        ),
                        channel
                            .get("bit")
                            .map_or_else(|| self.span(None), |item| self.item_span(item)),
                    );
                }
            }
        }
    }

    fn parse_dc(&mut self, device: &str, item: &Item) {
        let Some(dc) = item.as_table_like() else {
            self.error(
                "STC-E-PROJECT-009",
                format!("profile device `{device}` `dc` must be a table"),
                self.item_span(item),
            );
            return;
        };
        self.check_unknown(dc, "dc", DC_KEYS);
        if let Some(value) = dc.get("assign-activate")
            && value.as_bool().is_none()
        {
            self.error(
                "STC-E-PROJECT-009",
                format!("profile device `{device}` `dc.assign-activate` must be a boolean"),
                self.item_span(value),
            );
        }
        for key in ["sync0-cycle", "sync0-shift", "sync1-cycle", "sync1-shift"] {
            if let Some(value) = dc.get(key)
                && value.as_integer().is_none_or(|value| value < 0)
            {
                self.error(
                    "STC-E-PROJECT-009",
                    format!("profile device `{device}` `dc.{key}` must be a non-negative integer"),
                    self.item_span(value),
                );
            }
        }
    }

    fn parse_routes(&mut self, item: &Item) -> Vec<ProfileRoute> {
        let Some(routes) = item.as_array_of_tables() else {
            self.error(
                "STC-E-PROJECT-009",
                "`routes` must be an array of `[[routes]]` tables",
                self.item_span(item),
            );
            return Vec::new();
        };
        let mut parsed = Vec::new();
        let mut sinks: BTreeMap<String, Span> = BTreeMap::new();
        for route in routes {
            let span = self.table_span(route);
            self.check_unknown(route, "route", ROUTE_KEYS);
            let Some(source_item) = route.get("source") else {
                self.error(
                    "STC-E-IO-BINDING-002",
                    "route is missing its `source` endpoint",
                    span,
                );
                continue;
            };
            let source = match self.parse_endpoint(source_item, true) {
                Some(endpoint) => endpoint,
                None => continue,
            };
            let Some(sinks_item) = route.get("sinks").and_then(Item::as_array) else {
                self.error(
                    "STC-E-IO-BINDING-002",
                    "route `sinks` must be a non-empty array of endpoint objects",
                    route.get("sinks").map_or(span, |item| self.item_span(item)),
                );
                continue;
            };
            if sinks_item.is_empty() {
                self.error(
                    "STC-E-IO-BINDING-002",
                    "route `sinks` must contain at least one endpoint",
                    self.item_span(route.get("sinks").unwrap_or(item)),
                );
                continue;
            }
            let mut route_sinks = Vec::new();
            for sink_item in sinks_item {
                let Some(sink) = self.parse_endpoint_value(sink_item, false) else {
                    continue;
                };
                let key = sink_key(&sink);
                if let Some(previous) = sinks.get(&key) {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "STC-E-IO-BINDING-001",
                            format!(
                                "route sink `{}` already has a source",
                                describe_endpoint(&sink)
                            ),
                            sink.span(),
                        )
                        .with_label(*previous, "first source here"),
                    );
                } else {
                    sinks.insert(key, sink.span());
                }
                route_sinks.push(sink);
            }
            parsed.push(ProfileRoute {
                source,
                sinks: route_sinks,
                span,
            });
        }
        parsed
    }

    fn parse_endpoint(&mut self, item: &Item, source: bool) -> Option<ProfileEndpoint> {
        if let Some(table) = item.as_table_like() {
            return self.parse_endpoint_table(table, item, source);
        }
        self.error(
            "STC-E-IO-BINDING-002",
            "route endpoint must be an inline table or table",
            self.item_span(item),
        );
        None
    }

    fn parse_endpoint_value(&mut self, value: &Value, source: bool) -> Option<ProfileEndpoint> {
        let table = match value {
            Value::InlineTable(table) => table as &dyn TableLike,
            _ => {
                self.error(
                    "STC-E-IO-BINDING-002",
                    "route endpoint must be an inline table such as \
                     `{ kind = \"plc-input\", variable = \"Main.In0\" }`",
                    self.value_span(value),
                );
                return None;
            }
        };
        // Inline-table endpoints share the item parsing path.
        let item = Item::Value(value.clone());
        self.parse_endpoint_table(table, &item, source)
    }

    fn parse_endpoint_table(
        &mut self,
        table: &dyn TableLike,
        item: &Item,
        source: bool,
    ) -> Option<ProfileEndpoint> {
        let span = self.item_span(item);
        let Some(kind) = table.get("kind").and_then(Item::as_str) else {
            self.error(
                "STC-E-IO-BINDING-002",
                "route endpoint is missing its `kind` string",
                span,
            );
            return None;
        };
        let physical = matches!(kind, "physical-input" | "physical-output");
        let plc = matches!(kind, "plc-input" | "plc-output");
        if !physical && !plc {
            self.error(
                "STC-E-IO-BINDING-002",
                format!(
                    "unknown route endpoint kind `{kind}`; expected `physical-input`, \
                     `plc-output`, `plc-input`, or `physical-output`"
                ),
                span,
            );
            return None;
        }
        self.check_unknown(
            table,
            "route endpoint",
            if physical {
                PHYSICAL_ENDPOINT_KEYS
            } else {
                PLC_ENDPOINT_KEYS
            },
        );
        if source && matches!(kind, "plc-input" | "physical-output") {
            self.error(
                "STC-E-IO-BINDING-002",
                format!("route source kind `{kind}` is not allowed; a source is `physical-input` or `plc-output`"),
                span,
            );
            return None;
        }
        if !source && matches!(kind, "physical-input" | "plc-output") {
            self.error(
                "STC-E-IO-BINDING-002",
                format!("route sink kind `{kind}` is not allowed; a sink is `plc-input` or `physical-output`"),
                span,
            );
            return None;
        }
        if physical {
            let Some(device) = table.get("device").and_then(Item::as_str) else {
                self.error(
                    "STC-E-IO-BINDING-002",
                    format!("route endpoint `{kind}` is missing its `device` string"),
                    span,
                );
                return None;
            };
            let Some(channel) = table.get("channel").and_then(Item::as_str) else {
                self.error(
                    "STC-E-IO-BINDING-002",
                    format!("route endpoint `{kind}` is missing its `channel` string"),
                    span,
                );
                return None;
            };
            return Some(if kind == "physical-input" {
                ProfileEndpoint::PhysicalInput {
                    device: device.to_owned(),
                    channel: channel.to_owned(),
                    span,
                }
            } else {
                ProfileEndpoint::PhysicalOutput {
                    device: device.to_owned(),
                    channel: channel.to_owned(),
                    span,
                }
            });
        }
        let Some(variable) = table.get("variable").and_then(Item::as_str) else {
            self.error(
                "STC-E-IO-BINDING-002",
                format!("route endpoint `{kind}` is missing its `variable` string"),
                span,
            );
            return None;
        };
        Some(if kind == "plc-input" {
            ProfileEndpoint::PlcInput {
                variable: variable.to_owned(),
                span,
            }
        } else {
            ProfileEndpoint::PlcOutput {
                variable: variable.to_owned(),
                span,
            }
        })
    }

    fn config_table(&mut self, table: &dyn TableLike) -> ConfigValue {
        let mut values = BTreeMap::new();
        for (key, item) in table.iter() {
            if let Some(value) = self.config_item(item, 0) {
                values.insert(key.to_owned(), value);
            }
        }
        ConfigValue::Table(values)
    }

    fn config_item(&mut self, item: &Item, depth: usize) -> Option<ConfigValue> {
        if depth > CONFIG_NESTING_LIMIT {
            self.error(
                "STC-E-PROJECT-009",
                "profile device configuration is nested too deeply",
                self.item_span(item),
            );
            return None;
        }
        match item {
            Item::None => None,
            Item::Value(value) => self.config_value(value, depth),
            Item::Table(table) => {
                let mut values = BTreeMap::new();
                for (key, item) in table.iter() {
                    if let Some(value) = self.config_item(item, depth + 1) {
                        values.insert(key.to_owned(), value);
                    }
                }
                Some(ConfigValue::Table(values))
            }
            Item::ArrayOfTables(array) => {
                let mut values = Vec::with_capacity(array.len());
                for table in array {
                    let mut fields = BTreeMap::new();
                    for (key, item) in table.iter() {
                        if let Some(value) = self.config_item(item, depth + 1) {
                            fields.insert(key.to_owned(), value);
                        }
                    }
                    values.push(ConfigValue::Table(fields));
                }
                Some(ConfigValue::Array(values))
            }
        }
    }

    fn config_value(&mut self, value: &Value, depth: usize) -> Option<ConfigValue> {
        if depth > CONFIG_NESTING_LIMIT {
            self.error(
                "STC-E-PROJECT-009",
                "profile device configuration is nested too deeply",
                self.value_span(value),
            );
            return None;
        }
        Some(match value {
            Value::String(value) => ConfigValue::String(value.value().clone()),
            Value::Integer(value) => ConfigValue::Integer(*value.value()),
            Value::Float(value) => ConfigValue::Float(*value.value()),
            Value::Boolean(value) => ConfigValue::Boolean(*value.value()),
            Value::Datetime(value) => ConfigValue::Datetime(value.value().to_string()),
            Value::Array(values) => ConfigValue::Array(
                values
                    .iter()
                    .filter_map(|value| self.config_value(value, depth + 1))
                    .collect(),
            ),
            Value::InlineTable(table) => {
                let mut values = BTreeMap::new();
                for (key, value) in table.iter() {
                    if let Some(value) = self.config_value(value, depth + 1) {
                        values.insert(key.to_owned(), value);
                    }
                }
                ConfigValue::Table(values)
            }
        })
    }
}

/// Returns the declared `(index, subindex) -> bits` map for one device.
fn declared_entries(table: &dyn TableLike) -> BTreeMap<(u32, u8), u32> {
    let mut entries = BTreeMap::new();
    let Some(syncs) = table.get("syncs").and_then(Item::as_array_of_tables) else {
        return entries;
    };
    for sync in syncs {
        let Some(pdos) = sync.get("pdos").and_then(Item::as_array_of_tables) else {
            continue;
        };
        for pdo in pdos {
            let Some(items) = pdo.get("entries").and_then(Item::as_array_of_tables) else {
                continue;
            };
            for entry in items {
                let (Some(index), Some(subindex), Some(bits)) = (
                    entry.get("index").and_then(index_value),
                    entry.get("subindex").and_then(Item::as_integer),
                    entry.get("bits").and_then(Item::as_integer),
                ) else {
                    continue;
                };
                let Ok(subindex) = u8::try_from(subindex) else {
                    continue;
                };
                let Ok(bits) = u32::try_from(bits) else {
                    continue;
                };
                entries.insert((index, subindex), bits);
            }
        }
    }
    entries
}

/// Parses a PDO index or entry index written as an integer or hex string.
fn index_value(item: &Item) -> Option<u32> {
    if let Some(value) = item.as_integer() {
        return u32::try_from(value).ok();
    }
    let text = item.as_str()?;
    parse_identity(text)
}

/// Parses an integer, `0x`-prefixed hex, or bare hex identity string.
fn parse_identity(text: &str) -> Option<u32> {
    let text = text.trim();
    let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => (digits, 16),
        None => (text, 10),
    };
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

/// A stable sink identity shared by routes and bindings.
fn sink_key(endpoint: &ProfileEndpoint) -> String {
    match endpoint {
        ProfileEndpoint::PhysicalOutput {
            device, channel, ..
        } => {
            format!("output:{device}.{channel}")
        }
        ProfileEndpoint::PlcInput { variable, .. } => {
            format!("plc-input:{}", variable.to_ascii_uppercase())
        }
        ProfileEndpoint::PhysicalInput {
            device, channel, ..
        } => {
            // Sources are not sinks; the key is only used for diagnostics.
            format!("input:{device}.{channel}")
        }
        ProfileEndpoint::PlcOutput { variable, .. } => {
            format!("plc-output:{}", variable.to_ascii_uppercase())
        }
    }
}

fn describe_endpoint(endpoint: &ProfileEndpoint) -> String {
    match endpoint {
        ProfileEndpoint::PhysicalInput {
            device, channel, ..
        } => {
            format!("physical-input {device}.{channel}")
        }
        ProfileEndpoint::PhysicalOutput {
            device, channel, ..
        } => {
            format!("physical-output {device}.{channel}")
        }
        ProfileEndpoint::PlcInput { variable, .. } => format!("plc-input {variable}"),
        ProfileEndpoint::PlcOutput { variable, .. } => format!("plc-output {variable}"),
    }
}

fn render_keys(keys: &[&str]) -> String {
    keys.iter()
        .map(|key| format!("`{key}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: &str = r#"
[device.diewu]
master = 0
alias = 0
position = 0
vendor = "0x00000b95"
product = "0x00010200"
revision = 1
startup-timeout = "5s"
recovery-cycles = 3

[[device.diewu.syncs]]
index = 0
direction = "output"
watchdog = "enable"

[[device.diewu.syncs.pdos]]
index = "0x1a00"

[[device.diewu.syncs.pdos.entries]]
index = "0x3101"
subindex = 1
bits = 8

[[device.diewu.pdo-channels]]
name = "OUT0"
entry = { index = "0x3101", subindex = 1 }
bit = 0
"#;

    fn file() -> FileId {
        FileId::new(0)
    }

    #[test]
    fn parses_a_device_tree_and_routes() {
        let text = format!(
            "{DEVICE}\n\
             [[routes]]\n\
             source = {{ kind = \"physical-input\", device = \"diewu\", channel = \"IN0\" }}\n\
             sinks = [\n\
               {{ kind = \"plc-input\", variable = \"Main.In0\" }},\n\
               {{ kind = \"physical-output\", device = \"diewu\", channel = \"OUT1\" }},\n\
             ]\n"
        );
        let profile = parse_profile(&text, file()).expect("valid profile");
        assert_eq!(profile.devices.len(), 1);
        assert_eq!(profile.devices[0].name, "diewu");
        let ConfigValue::Table(table) = &profile.devices[0].config else {
            panic!("device config is not a table");
        };
        assert_eq!(table.get("master"), Some(&ConfigValue::Integer(0)));
        assert_eq!(profile.routes.len(), 1);
        assert!(matches!(
            profile.routes[0].source,
            ProfileEndpoint::PhysicalInput { .. }
        ));
        assert_eq!(profile.routes[0].sinks.len(), 2);
    }

    #[test]
    fn fans_out_but_rejects_fan_in() {
        let fan_out = format!(
            "{DEVICE}\n\
             [[routes]]\n\
             source = {{ kind = \"plc-output\", variable = \"Main.Out\" }}\n\
             sinks = [\n\
               {{ kind = \"physical-output\", device = \"diewu\", channel = \"OUT0\" }},\n\
               {{ kind = \"physical-output\", device = \"diewu\", channel = \"OUT1\" }},\n\
             ]\n"
        );
        parse_profile(&fan_out, file()).expect("fan-out is valid");

        let fan_in = format!(
            "{DEVICE}\n\
             [[routes]]\n\
             source = {{ kind = \"plc-output\", variable = \"Main.Out\" }}\n\
             sinks = [{{ kind = \"physical-output\", device = \"diewu\", channel = \"OUT0\" }}]\n\
             [[routes]]\n\
             source = {{ kind = \"plc-output\", variable = \"Main.Out2\" }}\n\
             sinks = [{{ kind = \"physical-output\", device = \"diewu\", channel = \"OUT0\" }}]\n"
        );
        let diagnostics = parse_profile(&fan_in, file()).expect_err("fan-in is rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "STC-E-IO-BINDING-001"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn rejects_invalid_endpoint_pairs_and_shapes() {
        let cases = [
            // Physical input cannot be a sink.
            r#"
[[routes]]
source = { kind = "plc-output", variable = "Main.Out" }
sinks = [{ kind = "physical-input", device = "d", channel = "IN0" }]
"#,
            // PLC input cannot be a source.
            r#"
[[routes]]
source = { kind = "plc-input", variable = "Main.In" }
sinks = [{ kind = "plc-output", variable = "Main.Out" }]
"#,
            // A sink object with the wrong key set.
            r#"
[[routes]]
source = { kind = "plc-output", variable = "Main.Out" }
sinks = [{ kind = "physical-output", variable = "Main.Out" }]
"#,
            // Empty sink list.
            r#"
[[routes]]
source = { kind = "plc-output", variable = "Main.Out" }
sinks = []
"#,
        ];
        for text in cases {
            assert!(
                parse_profile(text, file()).is_err(),
                "accepted invalid route: {text}"
            );
        }
    }

    #[test]
    fn rejects_unknown_entries_and_bad_pdo_channels() {
        let undeclared = r#"
[device.diewu]
[[device.diewu.pdo-channels]]
name = "OUT0"
entry = { index = "0x3101", subindex = 1 }
bit = 0
"#;
        assert!(parse_profile(undeclared, file()).is_err());

        let outside = r#"
[device.diewu]
[[device.diewu.syncs]]
index = 0
direction = "output"
watchdog = "enable"
[[device.diewu.syncs.pdos]]
index = "0x1a00"
[[device.diewu.syncs.pdos.entries]]
index = "0x3101"
subindex = 1
bits = 8
[[device.diewu.pdo-channels]]
name = "OUT0"
entry = { index = "0x3101", subindex = 1 }
bit = 8
"#;
        assert!(parse_profile(outside, file()).is_err());
    }

    #[test]
    fn rejects_duplicate_entries_and_device_keys() {
        let duplicate = r#"
[device.diewu]
[[device.diewu.syncs]]
index = 0
direction = "output"
watchdog = "enable"
[[device.diewu.syncs.pdos]]
index = "0x1a00"
[[device.diewu.syncs.pdos.entries]]
index = "0x3101"
subindex = 1
bits = 8
[[device.diewu.syncs.pdos.entries]]
index = "0x3101"
subindex = 1
bits = 16
"#;
        assert!(parse_profile(duplicate, file()).is_err());

        let unknown = "[device.diewu]\nmaster = 0\nnonsense = 1\n";
        assert!(parse_profile(unknown, file()).is_err());
    }

    #[test]
    fn normalizes_hex_identities_and_parses_inline_tables() {
        let text = r#"
[device.diewu]
vendor = "0x00000b95"
product = 66048
revision = "1"
dc = { assign-activate = true, sync0-cycle = 1000000 }
"#;
        let profile = parse_profile(text, file()).expect("valid device");
        let ConfigValue::Table(table) = &profile.devices[0].config else {
            panic!("device config is not a table");
        };
        assert_eq!(
            table.get("vendor"),
            Some(&ConfigValue::String("0x00000b95".into()))
        );
        let Some(ConfigValue::Table(dc)) = table.get("dc") else {
            panic!("dc is not a table");
        };
        assert_eq!(dc.get("assign-activate"), Some(&ConfigValue::Boolean(true)));
    }

    #[test]
    fn rejects_malformed_toml() {
        let diagnostics = parse_profile("[device.", file()).expect_err("invalid TOML");
        assert_eq!(diagnostics[0].code, "STC-E-PROJECT-001");
    }
}
