//! MQTT 5.0 topic names, topic filters, and matching (WL-15 phase 1).
//!
//! A publish topic is a topic name: non-empty, no wildcard, at most 65 535
//! bytes. A subscription uses a topic filter, where `+` matches exactly one
//! level and `#` matches the remaining levels and must be the last level.
//! `$share/<group>/<filter>` selects a shared subscription; the group is not a
//! topic level and never participates in matching.
//!
//! The `$` rule is enforced: a wildcard first level does not match a topic
//! whose first level begins with `$`, so `#` does not receive `$SYS` traffic.

/// Maximum encoded length of a topic name or filter.
pub const MAX_TOPIC_BYTES: usize = 65_535;

/// Shared-subscription prefix.
const SHARED_PREFIX: &str = "$share/";

/// Returns `true` when `topic` is a valid topic name for a publish.
#[must_use]
pub fn is_valid_topic_name(topic: &str) -> bool {
    !topic.is_empty()
        && topic.len() <= MAX_TOPIC_BYTES
        && !topic.contains('#')
        && !topic.contains('+')
        && !topic.contains('\0')
}

/// Returns `true` when `filter` is a valid subscription topic filter.
///
/// A shared subscription must name a group and wrap a valid filter.
#[must_use]
pub fn is_valid_topic_filter(filter: &str) -> bool {
    if filter.is_empty() || filter.len() > MAX_TOPIC_BYTES || filter.contains('\0') {
        return false;
    }
    if let Some(rest) = filter.strip_prefix(SHARED_PREFIX) {
        let Some((group, inner)) = rest.split_once('/') else {
            return false;
        };
        if group.is_empty() || group.contains('+') || group.contains('#') || group.contains('/') {
            return false;
        }
        return !inner.is_empty() && valid_plain_filter(inner);
    }
    valid_plain_filter(filter)
}

fn valid_plain_filter(filter: &str) -> bool {
    let levels: Vec<&str> = filter.split('/').collect();
    for (index, level) in levels.iter().enumerate() {
        if level.contains('#') {
            if *level != "#" || index + 1 != levels.len() {
                return false;
            }
        } else if level.contains('+') && *level != "+" {
            return false;
        }
    }
    true
}

/// Returns the group and inner filter of a shared subscription.
#[must_use]
pub fn shared_filter(filter: &str) -> Option<(&str, &str)> {
    let rest = filter.strip_prefix(SHARED_PREFIX)?;
    let (group, inner) = rest.split_once('/')?;
    if group.is_empty() || inner.is_empty() {
        return None;
    }
    Some((group, inner))
}

/// Returns `true` when `filter` matches `topic`.
///
/// An invalid filter or topic never matches. A shared subscription matches on
/// its inner filter.
#[must_use]
pub fn matches(filter: &str, topic: &str) -> bool {
    if !is_valid_topic_filter(filter) || !is_valid_topic_name(topic) {
        return false;
    }
    let inner = shared_filter(filter).map_or(filter, |(_, inner)| inner);
    let filter_levels: Vec<&str> = inner.split('/').collect();
    let topic_levels: Vec<&str> = topic.split('/').collect();
    if topic.starts_with('$') && matches!(filter_levels.first(), Some(&"#") | Some(&"+")) {
        return false;
    }
    let mut index = 0;
    while index < filter_levels.len() {
        let level = filter_levels[index];
        if level == "#" {
            return true;
        }
        let Some(topic_level) = topic_levels.get(index) else {
            return false;
        };
        if level != "+" && level != *topic_level {
            return false;
        }
        index += 1;
    }
    index == topic_levels.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_names_reject_wildcards_and_empty_values() {
        assert!(is_valid_topic_name("line1/plc/speed"));
        assert!(is_valid_topic_name("/leading"));
        assert!(is_valid_topic_name("a"));
        assert!(!is_valid_topic_name(""));
        assert!(!is_valid_topic_name("line1/+/speed"));
        assert!(!is_valid_topic_name("line1/#"));
        assert!(!is_valid_topic_name("line1/\0/speed"));
        assert!(!is_valid_topic_name(&"a".repeat(MAX_TOPIC_BYTES + 1)));
    }

    #[test]
    fn filters_accept_wildcards_only_as_complete_levels() {
        assert!(is_valid_topic_filter("#"));
        assert!(is_valid_topic_filter("line1/+/speed"));
        assert!(is_valid_topic_filter("line1/#"));
        assert!(is_valid_topic_filter("line1/plc/#"));
        assert!(!is_valid_topic_filter(""));
        assert!(!is_valid_topic_filter("line1/#/speed"));
        assert!(!is_valid_topic_filter("line1/speed#"));
        assert!(!is_valid_topic_filter("line1/+speed"));
        assert!(!is_valid_topic_filter("line1/spe+ed"));
        assert!(!is_valid_topic_filter("a\0b"));
    }

    #[test]
    fn shared_filters_require_a_group_and_inner_filter() {
        assert_eq!(
            shared_filter("$share/plc/line1/+/speed"),
            Some(("plc", "line1/+/speed"))
        );
        assert!(is_valid_topic_filter("$share/plc/line1/#"));
        assert_eq!(shared_filter("line1/speed"), None);
        assert_eq!(shared_filter("$share//line1/speed"), None);
        assert_eq!(shared_filter("$share/plc"), None);
        assert_eq!(shared_filter("$share/plc/"), None);
        assert!(!is_valid_topic_filter("$share//line1"));
        assert!(!is_valid_topic_filter("$share/pl+c/line1"));
        assert!(!is_valid_topic_filter("$share/pl#c/line1"));
        assert!(!is_valid_topic_filter("$share/grp/"));
        assert!(!is_valid_topic_filter("$share/grp/line1/#/x"));
    }

    #[test]
    fn matching_handles_single_and_multi_level_wildcards() {
        assert!(matches("line1/speed", "line1/speed"));
        assert!(!matches("line1/speed", "line1/speed/actual"));
        assert!(matches("line1/+/speed", "line1/plc/speed"));
        assert!(!matches("line1/+/speed", "line1/a/b/speed"));
        assert!(matches("line1/#", "line1"));
        assert!(matches("line1/#", "line1/plc/speed"));
        assert!(matches("#", "a/b/c"));
        assert!(matches("+/speed", "line1/speed"));
        assert!(!matches("+/speed", "line1/plc/speed"));
    }

    #[test]
    fn wildcards_do_not_match_dollar_topics_at_the_first_level() {
        assert!(!matches("#", "$SYS/broker/uptime"));
        assert!(!matches("+/broker/uptime", "$SYS/broker/uptime"));
        assert!(matches("$SYS/#", "$SYS/broker/uptime"));
        assert!(matches("$SYS/+/uptime", "$SYS/broker/uptime"));
    }

    #[test]
    fn shared_subscriptions_match_on_the_inner_filter() {
        assert!(matches("$share/plc/line1/+/speed", "line1/plc/speed"));
        assert!(!matches("$share/plc/line1/+/speed", "line2/plc/speed"));
        assert!(!matches("$share/plc/#", "$SYS/broker/uptime"));
    }

    #[test]
    fn invalid_inputs_never_match() {
        assert!(!matches("line1/#/x", "line1/a/x"));
        assert!(!matches("line1/+", "line1"));
        assert!(!matches("line1/speed", "line1/speed#"));
        assert!(!matches("", "line1"));
        assert!(!matches("line1", ""));
    }
}
