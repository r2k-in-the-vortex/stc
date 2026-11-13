//! Connection lifecycle state machine and keepalive scheduler (WL-15 phase 1).
//!
//! The session is transport-free and clock-free: the caller supplies a
//! monotonic `now` and acts on the returned [`SessionAction`]s. That keeps
//! every transition deterministic in tests and lets phase 3 own the socket
//! thread.
//!
//! Lifecycle: `Idle` → `Connecting` (CONNECT sent, waiting for CONNACK) →
//! `Connected` → `Disconnecting` → `Closed`. A connect failure, keepalive
//! timeout, broker DISCONNECT, or transport loss enters `Backoff`, which retries
//! on the configured ladder forever (WL-15 Q7: an unreachable broker never
//! fails the run).
//!
//! Keepalive follows MQTT 5.0: with a non-zero keepalive the client sends
//! PINGREQ when no packet has been sent for the interval and expects PINGRESP
//! within the next interval; otherwise the connection is considered dead.
//! A keepalive of zero disables the ping timers.

use std::collections::BTreeSet;
use std::time::Duration;

use super::packet::{Ack, Packet};
use super::{ErrorKind, MqttError};

/// Connection lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Not started.
    Idle,
    /// CONNECT sent, waiting for CONNACK.
    Connecting,
    /// CONNACK received; the session may exchange packets.
    Connected,
    /// Waiting for the next connection attempt.
    Backoff,
    /// DISCONNECT sent, waiting for the transport to close.
    Disconnecting,
    /// Terminal state; the session does not reconnect by itself.
    Closed,
}

impl SessionState {
    /// Returns the stable spelling used in diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Backoff => "backoff",
            Self::Disconnecting => "disconnecting",
            Self::Closed => "closed",
        }
    }
}

/// One thing the transport owner must do.
///
// The action carries an owned packet that the caller moves directly into its
// encoder. Boxing it would add one allocation per outbound message, which is
// the wrong trade on a cycle-adjacent path.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionAction {
    /// Open the transport and send the CONNECT packet.
    StartConnect,
    /// Abandon the current connection attempt and close the transport.
    AbortConnect,
    /// Encode and send this packet.
    Send(Packet),
    /// Call [`Session::on_tick`] again after this delay.
    Wait(Duration),
    /// The session is finished; join the transport thread.
    Closed,
}

/// Session timing configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// Keepalive interval; zero disables keepalive pings.
    pub keep_alive: Duration,
    /// Bound on the CONNECT/CONNACK exchange and on the disconnect flush.
    pub connect_timeout: Duration,
    /// Retry ladder; the last entry repeats.
    pub backoff: Vec<Duration>,
}

impl SessionConfig {
    /// Creates a validated configuration.
    ///
    /// # Errors
    ///
    /// Rejects an empty backoff ladder and a zero connect timeout; the profile
    /// validator reports both with a configuration diagnostic.
    pub fn new(
        keep_alive: Duration,
        connect_timeout: Duration,
        backoff: Vec<Duration>,
    ) -> Result<Self, MqttError> {
        if backoff.is_empty() {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "the reconnect backoff ladder must not be empty",
            ));
        }
        if connect_timeout.is_zero() {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                "the connect timeout must be greater than zero",
            ));
        }
        Ok(Self {
            keep_alive,
            connect_timeout,
            backoff,
        })
    }

    fn retry_delay(&self, index: usize) -> Duration {
        let last = self.backoff.len().saturating_sub(1);
        self.backoff
            .get(index.min(last))
            .copied()
            .unwrap_or(Duration::from_secs(1))
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            keep_alive: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(5),
            backoff: vec![
                Duration::from_millis(250),
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(5),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Deadline {
    /// Waiting for CONNACK.
    Connect,
    /// Waiting for the retry delay to elapse.
    Retry,
    /// Keepalive interval elapsed; ping when the deadline fires.
    KeepAlive,
    /// PINGREQ sent; waiting for PINGRESP.
    PingResponse,
    /// DISCONNECT sent; waiting for the transport to close.
    Disconnect,
}

/// The connection state machine.
///
/// The session does not encode or send anything itself; it returns
/// [`SessionAction::Send`] so the caller owns the transport and the timing.
#[derive(Clone, Debug)]
pub struct Session {
    config: SessionConfig,
    state: SessionState,
    deadline: Option<Duration>,
    deadline_kind: Option<Deadline>,
    backoff_index: usize,
    ping_outstanding: bool,
    session_present: bool,
    next_packet_id: u16,
    in_flight: BTreeSet<u16>,
    inbound_qos2: BTreeSet<u16>,
}

impl Session {
    /// Creates an idle session.
    #[must_use]
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            state: SessionState::Idle,
            deadline: None,
            deadline_kind: None,
            backoff_index: 0,
            ping_outstanding: false,
            session_present: false,
            next_packet_id: 1,
            in_flight: BTreeSet::new(),
            inbound_qos2: BTreeSet::new(),
        }
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns whether the broker continued an existing session.
    #[must_use]
    pub const fn session_present(&self) -> bool {
        self.session_present
    }

    /// Returns the current retry attempt index.
    #[must_use]
    pub const fn retry_index(&self) -> usize {
        self.backoff_index
    }

    /// Returns the number of unacknowledged outbound QoS 1/2 messages.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Returns the next scheduled deadline, when one exists.
    #[must_use]
    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// Starts or restarts the connection attempt.
    ///
    /// A session in `Backoff` cancels the remaining delay and connects now.
    /// A `Connected`, `Disconnecting`, or `Closed` session is left alone.
    pub fn start(&mut self, now: Duration) -> Vec<SessionAction> {
        match self.state {
            SessionState::Connected | SessionState::Disconnecting | SessionState::Closed => {
                Vec::new()
            }
            SessionState::Idle | SessionState::Connecting | SessionState::Backoff => {
                self.begin_connect(now)
            }
        }
    }

    /// Handles a decoded server packet.
    ///
    /// # Errors
    ///
    /// Rejects a packet that is not legal in the current state, and a CONNACK
    /// whose reason code is inconsistent with the session.
    pub fn on_packet(
        &mut self,
        now: Duration,
        packet: Packet,
    ) -> Result<Vec<SessionAction>, MqttError> {
        if !matches!(
            self.state,
            SessionState::Connecting | SessionState::Connected
        ) {
            return Err(MqttError::new(
                ErrorKind::Protocol,
                format!(
                    "received {} while {}",
                    packet.packet_type().as_str(),
                    self.state.as_str()
                ),
            ));
        }
        match packet {
            Packet::ConnAck(ack) => {
                if self.state != SessionState::Connecting {
                    return Err(MqttError::protocol("duplicate CONNACK"));
                }
                if !ack.is_success() {
                    return Ok(self.enter_backoff(now));
                }
                self.state = SessionState::Connected;
                self.backoff_index = 0;
                self.session_present = ack.session_present;
                self.ping_outstanding = false;
                Ok(self.arm_keepalive(now))
            }
            Packet::Publish(publish) => {
                self.note_sent(now);
                let mut actions = Vec::new();
                if let Some(packet_id) = publish.packet_id {
                    match publish.qos {
                        super::packet::QoS::AtMostOnce => {}
                        super::packet::QoS::AtLeastOnce => {
                            actions.push(SessionAction::Send(Packet::PubAck(Ack {
                                packet_id,
                                reason: 0,
                            })));
                        }
                        super::packet::QoS::ExactlyOnce => {
                            if self.inbound_qos2.insert(packet_id) {
                                actions.push(SessionAction::Send(Packet::PubRec(Ack {
                                    packet_id,
                                    reason: 0,
                                })));
                            }
                        }
                    }
                }
                Ok(actions)
            }
            Packet::PubAck(ack) | Packet::PubComp(ack) => {
                self.release_packet_id(ack.packet_id);
                self.note_sent(now);
                Ok(Vec::new())
            }
            Packet::PubRec(ack) => {
                self.note_sent(now);
                Ok(vec![SessionAction::Send(Packet::PubRel(Ack {
                    packet_id: ack.packet_id,
                    reason: 0,
                }))])
            }
            Packet::PubRel(ack) => {
                self.inbound_qos2.remove(&ack.packet_id);
                self.note_sent(now);
                Ok(vec![SessionAction::Send(Packet::PubComp(Ack {
                    packet_id: ack.packet_id,
                    reason: 0,
                }))])
            }
            Packet::SubAck(_) | Packet::UnsubAck(_) => {
                self.note_sent(now);
                Ok(Vec::new())
            }
            Packet::PingResp => {
                self.ping_outstanding = false;
                Ok(self.arm_keepalive(now))
            }
            Packet::Disconnect(_) => Ok(self.enter_backoff(now)),
            Packet::Connect(_)
            | Packet::Subscribe(_)
            | Packet::Unsubscribe(_)
            | Packet::PingReq => Err(MqttError::protocol(format!(
                "server sent the client-only packet {}",
                packet.packet_type().as_str()
            ))),
        }
    }

    /// Fires due timers and returns the next actions.
    ///
    /// Call this when [`SessionAction::Wait`] elapses. It is safe to call at any
    /// time; an early call is a no-op.
    pub fn on_tick(&mut self, now: Duration) -> Vec<SessionAction> {
        let Some(deadline) = self.deadline else {
            return Vec::new();
        };
        if now < deadline {
            return Vec::new();
        }
        match self.deadline_kind {
            Some(Deadline::Connect | Deadline::PingResponse) => self.enter_backoff(now),
            Some(Deadline::KeepAlive) => {
                if self.ping_outstanding {
                    return self.enter_backoff(now);
                }
                self.ping_outstanding = true;
                self.deadline = Some(now + self.config.keep_alive);
                self.deadline_kind = Some(Deadline::PingResponse);
                vec![
                    SessionAction::Send(Packet::PingReq),
                    SessionAction::Wait(self.config.keep_alive),
                ]
            }
            Some(Deadline::Retry) => self.begin_connect(now),
            Some(Deadline::Disconnect) => self.close(),
            None => Vec::new(),
        }
    }

    /// Handles a transport close or error.
    ///
    /// A `Disconnecting` session becomes `Closed`; any other live session
    /// enters `Backoff`.
    pub fn on_transport_closed(&mut self, now: Duration) -> Vec<SessionAction> {
        match self.state {
            SessionState::Closed => Vec::new(),
            SessionState::Disconnecting => self.close(),
            SessionState::Idle
            | SessionState::Connecting
            | SessionState::Connected
            | SessionState::Backoff => self.enter_backoff(now),
        }
    }

    /// Requests a clean disconnect.
    ///
    /// A connected session sends DISCONNECT and waits for the transport to
    /// close; any other live state becomes `Closed` immediately.
    pub fn request_disconnect(&mut self, now: Duration) -> Vec<SessionAction> {
        match self.state {
            SessionState::Connected => {
                self.state = SessionState::Disconnecting;
                self.deadline = Some(now + self.config.connect_timeout);
                self.deadline_kind = Some(Deadline::Disconnect);
                self.ping_outstanding = false;
                vec![
                    SessionAction::Send(Packet::Disconnect(super::packet::Disconnect {
                        reason: 0,
                        properties: super::packet::Properties::default(),
                    })),
                    SessionAction::Wait(self.config.connect_timeout),
                ]
            }
            SessionState::Idle
            | SessionState::Connecting
            | SessionState::Backoff
            | SessionState::Disconnecting
            | SessionState::Closed => self.close(),
        }
    }

    /// Records that the caller sent a packet, restarting the keepalive interval.
    ///
    /// A ping already in flight keeps its response deadline; any other outbound
    /// packet proves the connection is alive.
    pub fn note_sent(&mut self, now: Duration) {
        if self.state == SessionState::Connected && !self.ping_outstanding {
            self.arm_keepalive(now);
        }
    }

    /// Allocates the next free packet identifier.
    ///
    /// # Errors
    ///
    /// Returns `Exhausted` when every identifier is in flight.
    pub fn allocate_packet_id(&mut self) -> Result<u16, MqttError> {
        for _ in 0..u16::MAX {
            let candidate = self.next_packet_id;
            self.next_packet_id = if candidate == u16::MAX {
                1
            } else {
                candidate + 1
            };
            if self.in_flight.insert(candidate) {
                return Ok(candidate);
            }
        }
        Err(MqttError::exhausted(
            "no packet identifier is available; 65 535 messages are in flight",
        ))
    }

    /// Releases a packet identifier after its acknowledgement.
    pub fn release_packet_id(&mut self, packet_id: u16) {
        self.in_flight.remove(&packet_id);
        self.inbound_qos2.remove(&packet_id);
    }

    fn begin_connect(&mut self, now: Duration) -> Vec<SessionAction> {
        self.state = SessionState::Connecting;
        self.deadline = Some(now + self.config.connect_timeout);
        self.deadline_kind = Some(Deadline::Connect);
        self.ping_outstanding = false;
        vec![
            SessionAction::StartConnect,
            SessionAction::Wait(self.config.connect_timeout),
        ]
    }

    fn enter_backoff(&mut self, now: Duration) -> Vec<SessionAction> {
        let delay = self.config.retry_delay(self.backoff_index);
        self.backoff_index = (self.backoff_index + 1).min(self.config.backoff.len().saturating_sub(1));
        self.state = SessionState::Backoff;
        self.deadline = Some(now + delay);
        self.deadline_kind = Some(Deadline::Retry);
        self.ping_outstanding = false;
        vec![SessionAction::AbortConnect, SessionAction::Wait(delay)]
    }

    fn close(&mut self) -> Vec<SessionAction> {
        self.state = SessionState::Closed;
        self.deadline = None;
        self.deadline_kind = None;
        self.ping_outstanding = false;
        self.in_flight.clear();
        self.inbound_qos2.clear();
        vec![SessionAction::Closed]
    }

    fn arm_keepalive(&mut self, now: Duration) -> Vec<SessionAction> {
        if self.config.keep_alive.is_zero() {
            self.deadline = None;
            self.deadline_kind = None;
            return Vec::new();
        }
        self.deadline = Some(now + self.config.keep_alive);
        self.deadline_kind = Some(Deadline::KeepAlive);
        vec![SessionAction::Wait(self.config.keep_alive)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io_mqtt::packet::{ConnAck, Disconnect, Properties, QoS, Publish};

    fn session() -> Session {
        Session::new(
            SessionConfig::new(
                Duration::from_secs(30),
                Duration::from_secs(5),
                vec![
                    Duration::from_millis(250),
                    Duration::from_millis(500),
                    Duration::from_secs(1),
                ],
            )
            .unwrap(),
        )
    }

    fn connack(reason: u8, session_present: bool) -> Packet {
        Packet::ConnAck(ConnAck {
            session_present,
            reason,
            properties: Properties::default(),
        })
    }

    #[test]
    fn start_enters_connecting_with_a_connect_deadline() {
        let mut session = session();
        assert_eq!(session.state(), SessionState::Idle);
        let actions = session.start(Duration::ZERO);
        assert_eq!(
            actions,
            vec![
                SessionAction::StartConnect,
                SessionAction::Wait(Duration::from_secs(5))
            ]
        );
        assert_eq!(session.state(), SessionState::Connecting);
        assert_eq!(session.deadline(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn successful_connack_connects_and_reports_session_state() {
        let mut session = session();
        session.start(Duration::ZERO);
        let actions = session.on_packet(Duration::from_millis(10), connack(0, true)).unwrap();
        assert_eq!(session.state(), SessionState::Connected);
        assert!(session.session_present());
        assert_eq!(session.retry_index(), 0);
        assert_eq!(
            actions,
            vec![SessionAction::Wait(Duration::from_secs(30))]
        );
    }

    #[test]
    fn failed_connack_backs_off_on_the_ladder() {
        let mut session = session();
        session.start(Duration::ZERO);
        let actions = session
            .on_packet(Duration::from_millis(10), connack(0x88, false))
            .unwrap();
        assert_eq!(session.state(), SessionState::Backoff);
        assert_eq!(
            actions,
            vec![
                SessionAction::AbortConnect,
                SessionAction::Wait(Duration::from_millis(250))
            ]
        );
        // Zero actions before the deadline, then a new connect attempt.
        assert!(session.on_tick(Duration::from_millis(100)).is_empty());
        let actions = session.on_tick(Duration::from_millis(260));
        assert_eq!(
            actions,
            vec![
                SessionAction::StartConnect,
                SessionAction::Wait(Duration::from_secs(5))
            ]
        );
        assert_eq!(session.state(), SessionState::Connecting);
    }

    #[test]
    fn backoff_ladder_caps_at_the_last_entry() {
        let mut session = session();
        session.start(Duration::ZERO);
        let mut now = Duration::ZERO;
        let mut delays = Vec::new();
        for _ in 0..5 {
            let actions = session.on_packet(now, connack(0x80, false)).unwrap();
            let SessionAction::Wait(delay) = actions[1] else {
                panic!("expected a wait action");
            };
            delays.push(delay);
            now += delay;
            session.on_tick(now);
        }
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(250),
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ]
        );
    }

    #[test]
    fn connect_timeout_without_connack_backs_off() {
        let mut session = session();
        session.start(Duration::ZERO);
        assert!(session.on_tick(Duration::from_millis(4999)).is_empty());
        let actions = session.on_tick(Duration::from_secs(5));
        assert_eq!(actions[0], SessionAction::AbortConnect);
        assert_eq!(session.state(), SessionState::Backoff);
    }

    #[test]
    fn keepalive_sends_pingreq_then_times_out() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        assert!(session.on_tick(Duration::from_secs(29)).is_empty());
        let actions = session.on_tick(Duration::from_secs(30));
        assert_eq!(
            actions,
            vec![
                SessionAction::Send(Packet::PingReq),
                SessionAction::Wait(Duration::from_secs(30))
            ]
        );
        // Waiting longer than the keepalive without PINGRESP ends the session.
        let actions = session.on_tick(Duration::from_secs(60));
        assert_eq!(actions[0], SessionAction::AbortConnect);
        assert_eq!(session.state(), SessionState::Backoff);
    }

    #[test]
    fn pingresp_keeps_the_session_alive() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        session.on_tick(Duration::from_secs(30));
        let actions = session
            .on_packet(Duration::from_secs(31), Packet::PingResp)
            .unwrap();
        assert_eq!(session.state(), SessionState::Connected);
        assert_eq!(
            actions,
            vec![SessionAction::Wait(Duration::from_secs(30))]
        );
    }

    #[test]
    fn outbound_packets_postpone_the_keepalive() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        session.note_sent(Duration::from_secs(20));
        assert_eq!(session.deadline(), Some(Duration::from_secs(50)));
        assert!(session.on_tick(Duration::from_secs(30)).is_empty());
    }

    #[test]
    fn zero_keepalive_disables_ping_timers() {
        let mut session = Session::new(
            SessionConfig::new(Duration::ZERO, Duration::from_secs(5), vec![Duration::from_secs(1)])
                .unwrap(),
        );
        session.start(Duration::ZERO);
        let actions = session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        assert!(actions.is_empty());
        assert_eq!(session.deadline(), None);
    }

    #[test]
    fn inbound_publish_produces_acknowledgements() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        let qos1 = Packet::Publish(Publish {
            topic: "line1/speed".to_owned(),
            payload: vec![1],
            qos: QoS::AtLeastOnce,
            retain: false,
            dup: false,
            packet_id: Some(9),
            properties: Properties::default(),
        });
        assert_eq!(
            session.on_packet(Duration::from_secs(1), qos1).unwrap(),
            vec![SessionAction::Send(Packet::PubAck(Ack {
                packet_id: 9,
                reason: 0
            }))]
        );
        let qos2 = Packet::Publish(Publish {
            topic: "line1/speed".to_owned(),
            payload: vec![1],
            qos: QoS::ExactlyOnce,
            retain: false,
            dup: false,
            packet_id: Some(10),
            properties: Properties::default(),
        });
        assert_eq!(
            session.on_packet(Duration::from_secs(1), qos2.clone()).unwrap(),
            vec![SessionAction::Send(Packet::PubRec(Ack {
                packet_id: 10,
                reason: 0
            }))]
        );
        // A duplicate PUBLISH does not repeat the PUBREC.
        assert!(session
            .on_packet(Duration::from_secs(1), qos2)
            .unwrap()
            .is_empty());
        assert_eq!(
            session
                .on_packet(
                    Duration::from_secs(1),
                    Packet::PubRel(Ack {
                        packet_id: 10,
                        reason: 0
                    })
                )
                .unwrap(),
            vec![SessionAction::Send(Packet::PubComp(Ack {
                packet_id: 10,
                reason: 0
            }))]
        );
    }

    #[test]
    fn qos2_outbound_flow_releases_the_packet_id() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        let id = session.allocate_packet_id().unwrap();
        assert_eq!(session.in_flight(), 1);
        let actions = session
            .on_packet(
                Duration::from_secs(1),
                Packet::PubRec(Ack {
                    packet_id: id,
                    reason: 0,
                }),
            )
            .unwrap();
        assert_eq!(
            actions,
            vec![SessionAction::Send(Packet::PubRel(Ack {
                packet_id: id,
                reason: 0
            }))]
        );
        session
            .on_packet(
                Duration::from_secs(1),
                Packet::PubComp(Ack {
                    packet_id: id,
                    reason: 0,
                }),
            )
            .unwrap();
        assert_eq!(session.in_flight(), 0);
    }

    #[test]
    fn packet_ids_skip_in_flight_values_and_wrap() {
        let mut session = session();
        let first = session.allocate_packet_id().unwrap();
        assert_eq!(first, 1);
        let second = session.allocate_packet_id().unwrap();
        assert_eq!(second, 2);
        session.release_packet_id(first);
        let third = session.allocate_packet_id().unwrap();
        assert_eq!(third, 3);
        session.next_packet_id = u16::MAX;
        assert_eq!(session.allocate_packet_id().unwrap(), u16::MAX);
        assert_eq!(session.allocate_packet_id().unwrap(), 1);
    }

    #[test]
    fn clean_disconnect_sends_disconnect_then_closes() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        let actions = session.request_disconnect(Duration::from_secs(1));
        assert_eq!(
            actions,
            vec![
                SessionAction::Send(Packet::Disconnect(Disconnect {
                    reason: 0,
                    properties: Properties::default()
                })),
                SessionAction::Wait(Duration::from_secs(5)),
            ]
        );
        assert_eq!(session.state(), SessionState::Disconnecting);
        let actions = session.on_tick(Duration::from_secs(6));
        assert_eq!(actions, vec![SessionAction::Closed]);
        assert_eq!(session.state(), SessionState::Closed);
        assert!(session.on_transport_closed(Duration::from_secs(7)).is_empty());
    }

    #[test]
    fn transport_loss_reconnects_from_any_live_state() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        let actions = session.on_transport_closed(Duration::from_secs(1));
        assert_eq!(actions[0], SessionAction::AbortConnect);
        assert_eq!(session.state(), SessionState::Backoff);
        // In-flight messages are not acknowledged by a lost transport.
        let id = session.allocate_packet_id().unwrap();
        assert_eq!(session.in_flight(), 1);
        session.on_transport_closed(Duration::from_secs(2));
        assert_eq!(session.in_flight(), 1);
        session.release_packet_id(id);
    }

    #[test]
    fn broker_disconnect_enters_backoff() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        let actions = session
            .on_packet(
                Duration::from_secs(1),
                Packet::Disconnect(Disconnect {
                    reason: 0x8E,
                    properties: Properties::default(),
                }),
            )
            .unwrap();
        assert_eq!(actions[0], SessionAction::AbortConnect);
        assert_eq!(session.state(), SessionState::Backoff);
    }

    #[test]
    fn unexpected_packets_are_rejected() {
        let mut session = session();
        assert!(
            session
                .on_packet(Duration::ZERO, Packet::PingResp)
                .unwrap_err()
                .kind()
                == ErrorKind::Protocol
        );
        session.start(Duration::ZERO);
        // A second CONNACK after the session is connected is a protocol error.
        session.on_packet(Duration::ZERO, connack(0, false)).unwrap();
        assert!(
            session
                .on_packet(Duration::from_secs(1), connack(0, false))
                .is_err()
        );
        assert!(
            session
                .on_packet(Duration::from_secs(1), Packet::PingReq)
                .is_err()
        );
    }

    #[test]
    fn backoff_retry_can_be_started_manually() {
        let mut session = session();
        session.start(Duration::ZERO);
        session.on_packet(Duration::ZERO, connack(0x80, false)).unwrap();
        let actions = session.start(Duration::from_millis(1));
        assert_eq!(
            actions,
            vec![
                SessionAction::StartConnect,
                SessionAction::Wait(Duration::from_secs(5))
            ]
        );
        assert_eq!(session.state(), SessionState::Connecting);
    }

    #[test]
    fn closed_sessions_ignore_lifecycle_requests() {
        let mut session = session();
        session.request_disconnect(Duration::ZERO);
        assert_eq!(session.state(), SessionState::Closed);
        assert!(session.start(Duration::from_secs(1)).is_empty());
        assert!(session.on_tick(Duration::from_secs(1)).is_empty());
        assert_eq!(session.request_disconnect(Duration::from_secs(1)).len(), 1);
    }

    #[test]
    fn config_rejects_empty_ladder_and_zero_timeout() {
        assert!(
            SessionConfig::new(Duration::from_secs(1), Duration::from_secs(1), Vec::new()).is_err()
        );
        assert!(
            SessionConfig::new(Duration::from_secs(1), Duration::ZERO, vec![Duration::from_secs(1)])
                .is_err()
        );
        let config = SessionConfig::default();
        assert_eq!(config.keep_alive, Duration::from_secs(30));
        assert_eq!(config.backoff.len(), 5);
    }
}
