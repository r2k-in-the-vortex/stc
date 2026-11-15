#![no_main]

//! Fuzz the MQTT 5.0 packet decoder and payload/topic helpers (WL-15).
//!
//! Arbitrary bytes reach the packet decoder with and without a packet-size
//! limit. A decoded packet must re-encode when it is a client packet, and any
//! error must be a typed `MqttError` rather than a panic. The target never
//! opens a socket or reads the clock.

use libfuzzer_sys::fuzz_target;
use stc_runtime::io_mqtt::packet::{Packet, decode_packet};
use stc_runtime::io_mqtt::payload::{ByteOrder, Encoding, decode};
use stc_runtime::io_mqtt::topic::{is_valid_topic_filter, is_valid_topic_name, matches};

fuzz_target!(|data: &[u8]| {
    for limit in [0_usize, 64, 4096] {
        if let Ok((packet, used)) = decode_packet(data, limit) {
            assert!(used <= data.len());
            if !matches!(
                packet,
                Packet::ConnAck(_) | Packet::SubAck(_) | Packet::UnsubAck(_) | Packet::PingResp
            ) {
                let encoded = packet.encode().expect("a decoded client packet re-encodes");
                let (again, reencoded) = decode_packet(&encoded, 0).expect("re-encoded packet decodes");
                assert_eq!(again, packet);
                assert_eq!(reencoded, encoded.len());
            }
        }
    }

    if let Ok(text) = std::str::from_utf8(data) {
        let _ = is_valid_topic_name(text);
        let _ = is_valid_topic_filter(text);
        let _ = matches(text, text);
    }

    for encoding in [
        Encoding::Bool,
        Encoding::Uint,
        Encoding::Int,
        Encoding::Float,
        Encoding::String,
        Encoding::Raw,
    ] {
        for width in [1_u32, 8, 16, 32, 64, 24] {
            for order in [ByteOrder::Big, ByteOrder::Little] {
                let _ = decode(encoding, width, order, data);
            }
        }
    }
});
