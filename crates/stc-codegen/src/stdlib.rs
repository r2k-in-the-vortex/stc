//! M5 phase 5: synthesized bodies for the standard function-block library.
//!
//! The prelude defines the standard FB types and their input/output/state
//! fields; this module emits their bodies directly. Timers read the host's
//! latched `cycle_now_ns` service so every timer in one scan observes the same
//! time (D-134).

use std::collections::BTreeMap;

use stc_got::catalog::ImportService;
use stc_hir::DefId;
use stc_span::{FileId, Span};

use crate::CodegenError;
use crate::codes;
use crate::expr::BodyInfo;
use crate::ir::{Function, IrType, SymbolAttrs, Value};
use crate::types::RootSegment;
use crate::unit::Lowerer;

impl Lowerer<'_> {
    /// Emits the body of one synthesized standard FB.
    pub(crate) fn emit_standard_fb_body(
        &mut self,
        def: DefId,
        name: &'static str,
    ) -> Result<(), CodegenError> {
        let symbol = self
            .function_symbols
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("standard FB body has no symbol"))?;
        let layout = self
            .objects
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("standard FB has no object layout"))?;
        let count = layout.leaves.len();
        let table_type = format!("[{count} x ptr]");
        let mut fields: BTreeMap<String, (u32, IrType)> = BTreeMap::new();
        for (index, leaf) in layout.leaves.iter().enumerate() {
            let [RootSegment::Field(field)] = leaf.segments.as_slice() else {
                continue;
            };
            let field_data = self.hir.field(*field);
            let field_name = self.hir.interner.text(field_data.name).to_ascii_uppercase();
            let storage = self.types.llvm_storage(field_data.ty)?;
            fields.insert(
                field_name,
                (u32::try_from(index).unwrap_or(u32::MAX), storage),
            );
        }
        let mut f = Function::new(
            symbol,
            SymbolAttrs::internal(),
            IrType::Void,
            vec![
                (IrType::Ptr, "%this".into()),
                (IrType::Ptr, "%table".into()),
                (IrType::Ptr, "%slots".into()),
            ],
        );
        f.start_fresh_block();
        let slots = f.param_value(2);
        let table = f.param_value(1);
        let mut pointers: BTreeMap<String, Value> = BTreeMap::new();
        for (field_name, (index, _)) in &fields {
            let entry = f.gep(
                &table_type,
                &table,
                &["i64 0".into(), format!("i64 {index}")],
                true,
            );
            pointers.insert(field_name.clone(), f.load(IrType::Ptr, &entry, Some(8)));
        }
        let info = BodyInfo {
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            local_objects: BTreeMap::new(),
            result_ptr: None,
            result_ty: None,
            slots,
            fields: None,
            handle: None,
            return_label: String::new(),
        };
        let now = if matches!(name, "TON" | "TOF" | "TP") {
            let slot = self.import_slot(ImportService::CycleNowNs)?;
            let service = Self::slot_storage(&mut f, &info, slot);
            Some(
                f.call_ptr(IrType::Int(64), &service, &[])
                    .ok_or_else(|| CodegenError::internal("cycle clock returned void"))?,
            )
        } else {
            None
        };
        match name {
            "TON" => emit_ton(&mut f, &pointers, &now.expect("timer clock"))?,
            "TOF" => emit_tof(&mut f, &pointers, &now.expect("timer clock"))?,
            "TP" => emit_tp(&mut f, &pointers, &now.expect("timer clock"))?,
            "R_TRIG" => emit_edge(&mut f, &pointers, true),
            "F_TRIG" => emit_edge(&mut f, &pointers, false),
            "CTU" => emit_ctu(&mut f, &pointers),
            "CTD" => emit_ctd(&mut f, &pointers),
            "CTUD" => emit_ctud(&mut f, &pointers),
            "SR" => emit_sr(&mut f, &pointers),
            "RS" => emit_rs(&mut f, &pointers),
            other => {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("standard FB `{other}` has no M5 body"),
                    Span::empty(FileId::new(0), 0),
                ));
            }
        }
        f.ret(None);
        f.check().map_err(CodegenError::internal)?;
        self.module.add_function(f);
        Ok(())
    }
}

fn field<'a>(fields: &'a BTreeMap<String, Value>, name: &str) -> Result<&'a Value, CodegenError> {
    fields
        .get(name)
        .ok_or_else(|| CodegenError::internal(format!("standard FB field `{name}` is missing")))
}

fn load_bool(f: &mut Function, pointer: &Value) -> Value {
    let raw = f.load(IrType::Int(8), pointer, None);
    f.icmp("ne", &raw, &Value::int(8, 0))
}

fn store_bool(f: &mut Function, pointer: &Value, value: &Value) {
    let stored = f.emit_value_cast("zext", value, IrType::Int(8));
    f.store(&stored, pointer, None);
}

fn load_time(f: &mut Function, pointer: &Value) -> Value {
    f.load(IrType::Int(64), pointer, None)
}

fn load_int(f: &mut Function, pointer: &Value) -> Value {
    f.load(IrType::Int(16), pointer, None)
}

fn emit_ton(
    f: &mut Function,
    fields: &BTreeMap<String, Value>,
    now: &Value,
) -> Result<(), CodegenError> {
    let input = load_bool(f, field(fields, "IN")?);
    let pt = load_time(f, field(fields, "PT")?);
    let end = f.next_block_label();
    let then_label = f.next_block_label();
    let else_label = f.next_block_label();
    f.cond_branch(&input, &then_label, &else_label);
    f.start_block(&then_label);
    let running = load_bool(f, field(fields, "__RUN")?);
    let start_label = f.next_block_label();
    let after_start = f.next_block_label();
    f.cond_branch(&running, &after_start, &start_label);
    f.start_block(&start_label);
    f.store(now, field(fields, "__START")?, None);
    store_bool(f, field(fields, "__RUN")?, &Value::bool(true));
    f.branch(&after_start);
    f.start_block(&after_start);
    let start = load_time(f, field(fields, "__START")?);
    let elapsed = f.binop("sub", now, &start);
    let timed_out = f.icmp("sge", &elapsed, &pt);
    let timeout_label = f.next_block_label();
    let running_label = f.next_block_label();
    f.cond_branch(&timed_out, &timeout_label, &running_label);
    f.start_block(&timeout_label);
    f.store(&pt, field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(true));
    f.branch(&end);
    f.start_block(&running_label);
    f.store(&elapsed, field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(false));
    f.branch(&end);
    f.start_block(&else_label);
    store_bool(f, field(fields, "__RUN")?, &Value::bool(false));
    f.store(&Value::int(64, 0), field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(false));
    f.branch(&end);
    f.start_block(&end);
    Ok(())
}

fn emit_tof(
    f: &mut Function,
    fields: &BTreeMap<String, Value>,
    now: &Value,
) -> Result<(), CodegenError> {
    let input = load_bool(f, field(fields, "IN")?);
    let pt = load_time(f, field(fields, "PT")?);
    let end = f.next_block_label();
    let then_label = f.next_block_label();
    let else_label = f.next_block_label();
    f.cond_branch(&input, &then_label, &else_label);
    f.start_block(&then_label);
    store_bool(f, field(fields, "Q")?, &Value::bool(true));
    f.store(&Value::int(64, 0), field(fields, "ET")?, None);
    store_bool(f, field(fields, "__RUN")?, &Value::bool(false));
    f.branch(&end);
    f.start_block(&else_label);
    let running = load_bool(f, field(fields, "__RUN")?);
    let start_label = f.next_block_label();
    let after_start = f.next_block_label();
    f.cond_branch(&running, &after_start, &start_label);
    f.start_block(&start_label);
    f.store(now, field(fields, "__START")?, None);
    store_bool(f, field(fields, "__RUN")?, &Value::bool(true));
    f.branch(&after_start);
    f.start_block(&after_start);
    let start = load_time(f, field(fields, "__START")?);
    let elapsed = f.binop("sub", now, &start);
    let expired = f.icmp("sge", &elapsed, &pt);
    let expired_label = f.next_block_label();
    let active_label = f.next_block_label();
    f.cond_branch(&expired, &expired_label, &active_label);
    f.start_block(&expired_label);
    f.store(&pt, field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(false));
    f.branch(&end);
    f.start_block(&active_label);
    f.store(&elapsed, field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(true));
    f.branch(&end);
    f.start_block(&end);
    Ok(())
}

fn emit_tp(
    f: &mut Function,
    fields: &BTreeMap<String, Value>,
    now: &Value,
) -> Result<(), CodegenError> {
    let input = load_bool(f, field(fields, "IN")?);
    let pt = load_time(f, field(fields, "PT")?);
    let end = f.next_block_label();
    let high_label = f.next_block_label();
    let low_label = f.next_block_label();
    f.cond_branch(&input, &high_label, &low_label);
    f.start_block(&low_label);
    store_bool(f, field(fields, "__RUN")?, &Value::bool(false));
    store_bool(f, field(fields, "__DONE")?, &Value::bool(false));
    f.store(&Value::int(64, 0), field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(false));
    f.branch(&end);
    f.start_block(&high_label);
    let running = load_bool(f, field(fields, "__RUN")?);
    let running_label = f.next_block_label();
    let idle_label = f.next_block_label();
    f.cond_branch(&running, &running_label, &idle_label);
    f.start_block(&running_label);
    let start = load_time(f, field(fields, "__START")?);
    let elapsed = f.binop("sub", now, &start);
    let expired = f.icmp("sge", &elapsed, &pt);
    let expired_label = f.next_block_label();
    let active_label = f.next_block_label();
    f.cond_branch(&expired, &expired_label, &active_label);
    f.start_block(&expired_label);
    f.store(&pt, field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(false));
    store_bool(f, field(fields, "__RUN")?, &Value::bool(false));
    store_bool(f, field(fields, "__DONE")?, &Value::bool(true));
    f.branch(&end);
    f.start_block(&active_label);
    f.store(&elapsed, field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(true));
    f.branch(&end);
    f.start_block(&idle_label);
    let done = load_bool(f, field(fields, "__DONE")?);
    let trigger_label = f.next_block_label();
    let waiting_label = f.next_block_label();
    f.cond_branch(&done, &waiting_label, &trigger_label);
    f.start_block(&trigger_label);
    f.store(now, field(fields, "__START")?, None);
    store_bool(f, field(fields, "__RUN")?, &Value::bool(true));
    f.store(&Value::int(64, 0), field(fields, "ET")?, None);
    store_bool(f, field(fields, "Q")?, &Value::bool(true));
    f.branch(&end);
    f.start_block(&waiting_label);
    store_bool(f, field(fields, "Q")?, &Value::bool(false));
    f.branch(&end);
    f.start_block(&end);
    Ok(())
}

fn emit_edge(f: &mut Function, fields: &BTreeMap<String, Value>, rising: bool) {
    let clock = load_bool(f, &fields["CLK"]);
    let previous = load_bool(f, &fields["__PREV"]);
    let not_previous = f.binop("xor", &previous, &Value::bool(true));
    let not_clock = f.binop("xor", &clock, &Value::bool(true));
    let edge = if rising {
        f.binop("and", &clock, &not_previous)
    } else {
        f.binop("and", &not_clock, &previous)
    };
    store_bool(f, &fields["Q"], &edge);
    store_bool(f, &fields["__PREV"], &clock);
}

fn emit_ctu(f: &mut Function, fields: &BTreeMap<String, Value>) {
    let cu = load_bool(f, &fields["CU"]);
    let reset = load_bool(f, &fields["R"]);
    let previous = load_bool(f, &fields["__PREV"]);
    let pv = load_int(f, &fields["PV"]);
    let cv = load_int(f, &fields["CV"]);
    let not_previous = f.binop("xor", &previous, &Value::bool(true));
    let rising = f.binop("and", &cu, &not_previous);
    let incremented = f.binop("add", &cv, &Value::int(16, 1));
    let counted = f.select(&rising, &incremented, &cv);
    let end = f.next_block_label();
    let reset_label = f.next_block_label();
    let count_label = f.next_block_label();
    f.cond_branch(&reset, &reset_label, &count_label);
    f.start_block(&reset_label);
    f.store(&Value::int(16, 0), &fields["CV"], None);
    f.branch(&end);
    f.start_block(&count_label);
    f.store(&counted, &fields["CV"], None);
    f.branch(&end);
    f.start_block(&end);
    store_bool(f, &fields["__PREV"], &cu);
    let current = load_int(f, &fields["CV"]);
    let q = f.icmp("sge", &current, &pv);
    store_bool(f, &fields["Q"], &q);
}

fn emit_ctd(f: &mut Function, fields: &BTreeMap<String, Value>) {
    let cd = load_bool(f, &fields["CD"]);
    let load = load_bool(f, &fields["LD"]);
    let previous = load_bool(f, &fields["__PREV"]);
    let pv = load_int(f, &fields["PV"]);
    let cv = load_int(f, &fields["CV"]);
    let not_previous = f.binop("xor", &previous, &Value::bool(true));
    let rising = f.binop("and", &cd, &not_previous);
    let decremented = f.binop("sub", &cv, &Value::int(16, 1));
    let counted = f.select(&rising, &decremented, &cv);
    let end = f.next_block_label();
    let load_label = f.next_block_label();
    let count_label = f.next_block_label();
    f.cond_branch(&load, &load_label, &count_label);
    f.start_block(&load_label);
    f.store(&pv, &fields["CV"], None);
    f.branch(&end);
    f.start_block(&count_label);
    f.store(&counted, &fields["CV"], None);
    f.branch(&end);
    f.start_block(&end);
    store_bool(f, &fields["__PREV"], &cd);
    let current = load_int(f, &fields["CV"]);
    let q = f.icmp("sle", &current, &Value::int(16, 0));
    store_bool(f, &fields["Q"], &q);
}

fn emit_ctud(f: &mut Function, fields: &BTreeMap<String, Value>) {
    let cu = load_bool(f, &fields["CU"]);
    let cd = load_bool(f, &fields["CD"]);
    let reset = load_bool(f, &fields["R"]);
    let load = load_bool(f, &fields["LD"]);
    let previous_up = load_bool(f, &fields["__PREVU"]);
    let previous_down = load_bool(f, &fields["__PREVD"]);
    let pv = load_int(f, &fields["PV"]);
    let cv = load_int(f, &fields["CV"]);
    let not_previous_up = f.binop("xor", &previous_up, &Value::bool(true));
    let not_previous_down = f.binop("xor", &previous_down, &Value::bool(true));
    let rising_up = f.binop("and", &cu, &not_previous_up);
    let rising_down = f.binop("and", &cd, &not_previous_down);
    let end = f.next_block_label();
    let reset_label = f.next_block_label();
    let load_test = f.next_block_label();
    let load_label = f.next_block_label();
    let update_label = f.next_block_label();
    let up_label = f.next_block_label();
    let after_up = f.next_block_label();
    let down_label = f.next_block_label();
    let after_down = f.next_block_label();
    f.cond_branch(&reset, &reset_label, &load_test);
    f.start_block(&reset_label);
    f.store(&Value::int(16, 0), &fields["CV"], None);
    f.branch(&end);
    f.start_block(&load_test);
    f.cond_branch(&load, &load_label, &update_label);
    f.start_block(&load_label);
    f.store(&pv, &fields["CV"], None);
    f.branch(&end);
    f.start_block(&update_label);
    f.cond_branch(&rising_up, &up_label, &after_up);
    f.start_block(&up_label);
    let incremented = f.binop("add", &cv, &Value::int(16, 1));
    f.branch(&after_up);
    f.start_block(&after_up);
    let up_value = f.phi(
        IrType::Int(16),
        &[(incremented, up_label), (cv.clone(), update_label)],
    );
    f.cond_branch(&rising_down, &down_label, &after_down);
    f.start_block(&down_label);
    let decremented = f.binop("sub", &up_value, &Value::int(16, 1));
    f.branch(&after_down);
    f.start_block(&after_down);
    let new_value = f.phi(
        IrType::Int(16),
        &[(decremented, down_label), (up_value, after_up)],
    );
    f.store(&new_value, &fields["CV"], None);
    f.branch(&end);
    f.start_block(&end);
    let current = load_int(f, &fields["CV"]);
    let qu = f.icmp("sge", &current, &pv);
    let qd = f.icmp("sle", &current, &Value::int(16, 0));
    store_bool(f, &fields["QU"], &qu);
    store_bool(f, &fields["QD"], &qd);
    store_bool(f, &fields["__PREVU"], &cu);
    store_bool(f, &fields["__PREVD"], &cd);
}

fn emit_sr(f: &mut Function, fields: &BTreeMap<String, Value>) {
    let set = load_bool(f, &fields["SET1"]);
    let reset = load_bool(f, &fields["RESET"]);
    let end = f.next_block_label();
    let set_label = f.next_block_label();
    let keep_label = f.next_block_label();
    let reset_label = f.next_block_label();
    f.cond_branch(&set, &set_label, &keep_label);
    f.start_block(&set_label);
    store_bool(f, &fields["Q"], &Value::bool(true));
    f.branch(&end);
    f.start_block(&keep_label);
    f.cond_branch(&reset, &reset_label, &end);
    f.start_block(&reset_label);
    store_bool(f, &fields["Q"], &Value::bool(false));
    f.branch(&end);
    f.start_block(&end);
}

fn emit_rs(f: &mut Function, fields: &BTreeMap<String, Value>) {
    let set = load_bool(f, &fields["SET"]);
    let reset = load_bool(f, &fields["RESET1"]);
    let end = f.next_block_label();
    let reset_label = f.next_block_label();
    let keep_label = f.next_block_label();
    let set_label = f.next_block_label();
    f.cond_branch(&reset, &reset_label, &keep_label);
    f.start_block(&reset_label);
    store_bool(f, &fields["Q"], &Value::bool(false));
    f.branch(&end);
    f.start_block(&keep_label);
    f.cond_branch(&set, &set_label, &end);
    f.start_block(&set_label);
    store_bool(f, &fields["Q"], &Value::bool(true));
    f.branch(&end);
    f.start_block(&end);
}
