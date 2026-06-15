//! 노드 메시지 enum + 본문 코덱 (개념: message). node-wire.md §4-5.

use crate::codec::{DecodeError, Reader, Writer};
use crate::frame::{Header, encode_frame};

/// `Option<String>` 인코딩: presence bool + (있으면) 길이접두사 문자열.
fn encode_opt_string(w: &mut Writer, s: &Option<String>) {
    match s {
        Some(v) => {
            w.bool(true);
            w.string(v);
        }
        None => w.bool(false),
    }
}

fn decode_opt_string(r: &mut Reader<'_>) -> Result<Option<String>, DecodeError> {
    Ok(if r.bool()? { Some(r.string()?) } else { None })
}

/// msg_type 상수.
pub mod msg_type {
    pub const HELLO: u16 = 0x0001;
    pub const HELLO_ACK: u16 = 0x0002;
    pub const PING: u16 = 0x0003;
    pub const PONG: u16 = 0x0004;
    pub const REALM_SEND: u16 = 0x0101;
    pub const REALM_FANOUT: u16 = 0x0103;
    pub const REALM_EMIT: u16 = 0x0104;
    pub const SUBSCRIBE: u16 = 0x0110;
    pub const UNSUBSCRIBE: u16 = 0x0111;
    pub const PRESENCE_GOSSIP: u16 = 0x0201;
    pub const USER_DELIVER: u16 = 0x0202;
}

/// 노드↔노드 메시지.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeMessage {
    Hello { capabilities: u32, epoch: u64 },
    HelloAck { capabilities: u32, epoch: u64 },
    Ping,
    Pong,
    /// 비소유 노드 → 소유 노드: 메시지 전송 위임 (D9). 메시지는 채널 단위.
    RealmSend {
        realm_id: u64,
        channel_id: u64,
        author: u64,
        content: String,
        nonce: Option<String>,
        /// 답장 대상 메시지 id (없으면 일반 메시지, D39).
        reference_message_id: Option<u64>,
    },
    /// 소유 노드 → 구독자 보유 노드: 이벤트 팬아웃 (범용 envelope, D39).
    /// `t`=DISPATCH 이벤트 이름, `payload`=직렬화된 JSON, `user_ids`=그 노드의 로컬 대상 (D12/D24).
    RealmFanout {
        realm_id: u64,
        t: String,
        payload: String,
        user_ids: Vec<u64>,
    },
    /// 비소유 노드 → 소유 노드: 비-메시지 이벤트 팬아웃 위임 (범용 envelope, D39).
    RealmEmit {
        realm_id: u64,
        t: String,
        payload: String,
    },
    Subscribe { realm_id: u64, user_id: u64, node_id: u64 },
    Unsubscribe { realm_id: u64, user_id: u64, node_id: u64 },
    /// 전역 presence 델타 (Q11/D12). `node_id`=이 유저를 호스팅(또는 해제)한 노드, `status`=user_status u8.
    /// 풀메시 브로드캐스트 → 각 노드가 자기 view 갱신 후 그 유저의 로컬 친구에게 PRESENCE_UPDATE 배달.
    PresenceGossip { user_id: u64, node_id: u64, status: u8 },
    /// 크로스노드 유저 이벤트 타깃 배달 (D43). Realm 무관 유저 이벤트(친구·읽음 등)를
    /// 대상 유저를 호스팅하는 노드에만 전송(broadcast 아님). 수신 노드가 로컬 세션에 배달.
    /// `t`=DISPATCH 이벤트 이름, `payload`=직렬화된 JSON(불투명), `user_ids`=이 노드의 로컬 대상.
    UserDeliver { t: String, payload: String, user_ids: Vec<u64> },
}

impl NodeMessage {
    pub fn msg_type(&self) -> u16 {
        match self {
            NodeMessage::Hello { .. } => msg_type::HELLO,
            NodeMessage::HelloAck { .. } => msg_type::HELLO_ACK,
            NodeMessage::Ping => msg_type::PING,
            NodeMessage::Pong => msg_type::PONG,
            NodeMessage::RealmSend { .. } => msg_type::REALM_SEND,
            NodeMessage::RealmFanout { .. } => msg_type::REALM_FANOUT,
            NodeMessage::RealmEmit { .. } => msg_type::REALM_EMIT,
            NodeMessage::Subscribe { .. } => msg_type::SUBSCRIBE,
            NodeMessage::Unsubscribe { .. } => msg_type::UNSUBSCRIBE,
            NodeMessage::PresenceGossip { .. } => msg_type::PRESENCE_GOSSIP,
            NodeMessage::UserDeliver { .. } => msg_type::USER_DELIVER,
        }
    }

    pub fn encode_body(&self, w: &mut Writer) {
        match self {
            NodeMessage::Hello { capabilities, epoch }
            | NodeMessage::HelloAck { capabilities, epoch } => {
                w.u32(*capabilities);
                w.u64(*epoch);
            }
            NodeMessage::Ping | NodeMessage::Pong => {}
            NodeMessage::RealmSend { realm_id, channel_id, author, content, nonce, reference_message_id } => {
                w.u64(*realm_id);
                w.u64(*channel_id);
                w.u64(*author);
                w.string(content);
                encode_opt_string(w, nonce);
                match reference_message_id {
                    Some(r) => {
                        w.bool(true);
                        w.u64(*r);
                    }
                    None => w.bool(false),
                }
            }
            NodeMessage::RealmFanout { realm_id, t, payload, user_ids } => {
                w.u64(*realm_id);
                w.string(t);
                w.string(payload);
                w.u32(user_ids.len() as u32);
                for u in user_ids {
                    w.u64(*u);
                }
            }
            NodeMessage::RealmEmit { realm_id, t, payload } => {
                w.u64(*realm_id);
                w.string(t);
                w.string(payload);
            }
            NodeMessage::Subscribe { realm_id, user_id, node_id }
            | NodeMessage::Unsubscribe { realm_id, user_id, node_id } => {
                w.u64(*realm_id);
                w.u64(*user_id);
                w.u64(*node_id);
            }
            NodeMessage::PresenceGossip { user_id, node_id, status } => {
                w.u64(*user_id);
                w.u64(*node_id);
                w.u8(*status);
            }
            NodeMessage::UserDeliver { t, payload, user_ids } => {
                w.string(t);
                w.string(payload);
                w.u32(user_ids.len() as u32);
                for u in user_ids {
                    w.u64(*u);
                }
            }
        }
    }

    pub fn decode_body(ty: u16, r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(match ty {
            msg_type::HELLO => NodeMessage::Hello { capabilities: r.u32()?, epoch: r.u64()? },
            msg_type::HELLO_ACK => NodeMessage::HelloAck { capabilities: r.u32()?, epoch: r.u64()? },
            msg_type::PING => NodeMessage::Ping,
            msg_type::PONG => NodeMessage::Pong,
            msg_type::REALM_SEND => {
                let realm_id = r.u64()?;
                let channel_id = r.u64()?;
                let author = r.u64()?;
                let content = r.string()?;
                let nonce = decode_opt_string(r)?;
                let reference_message_id = if r.bool()? { Some(r.u64()?) } else { None };
                NodeMessage::RealmSend { realm_id, channel_id, author, content, nonce, reference_message_id }
            }
            msg_type::REALM_FANOUT => {
                let realm_id = r.u64()?;
                let t = r.string()?;
                let payload = r.string()?;
                let n = r.u32()? as usize;
                let mut user_ids = Vec::with_capacity(n);
                for _ in 0..n {
                    user_ids.push(r.u64()?);
                }
                NodeMessage::RealmFanout { realm_id, t, payload, user_ids }
            }
            msg_type::REALM_EMIT => {
                let realm_id = r.u64()?;
                let t = r.string()?;
                let payload = r.string()?;
                NodeMessage::RealmEmit { realm_id, t, payload }
            }
            msg_type::SUBSCRIBE => NodeMessage::Subscribe {
                realm_id: r.u64()?,
                user_id: r.u64()?,
                node_id: r.u64()?,
            },
            msg_type::UNSUBSCRIBE => NodeMessage::Unsubscribe {
                realm_id: r.u64()?,
                user_id: r.u64()?,
                node_id: r.u64()?,
            },
            msg_type::PRESENCE_GOSSIP => NodeMessage::PresenceGossip {
                user_id: r.u64()?,
                node_id: r.u64()?,
                status: r.u8()?,
            },
            msg_type::USER_DELIVER => {
                let t = r.string()?;
                let payload = r.string()?;
                let n = r.u32()? as usize;
                let mut user_ids = Vec::with_capacity(n);
                for _ in 0..n {
                    user_ids.push(r.u64()?);
                }
                NodeMessage::UserDeliver { t, payload, user_ids }
            }
            other => return Err(DecodeError::UnknownTag(other)),
        })
    }

    /// 헤더+본문 → 길이접두사 프레임 (송신용 완성 바이트).
    pub fn encode(&self, src_node_id: u64, trace_id: u128) -> Vec<u8> {
        let mut w = Writer::new();
        Header::new(self.msg_type(), src_node_id, trace_id).encode(&mut w);
        self.encode_body(&mut w);
        encode_frame(&w.into_vec())
    }

    /// payload(헤더+본문, 길이접두사 제외) → (헤더, 메시지).
    pub fn decode(payload: &[u8]) -> Result<(Header, NodeMessage), DecodeError> {
        let mut r = Reader::new(payload);
        let h = Header::decode(&mut r)?;
        let m = NodeMessage::decode_body(h.msg_type, &mut r)?;
        Ok((h, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::read_frame;

    #[test]
    fn subscribe_round_trip_matches_doc_size() {
        let msg = NodeMessage::Subscribe { realm_id: 0x100, user_id: 0xA, node_id: 7 };
        let framed = msg.encode(7, 0);
        // node-wire.md §7 워크드 예시: 헤더28+본문24=52, 프레임 총 56바이트
        assert_eq!(framed.len(), 56);
        let (payload, consumed) = read_frame(&framed).unwrap().unwrap();
        assert_eq!(consumed, framed.len());
        let (h, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(h.msg_type, msg_type::SUBSCRIBE);
        assert_eq!(h.src_node_id, 7);
        assert_eq!(decoded, msg);
    }

    #[test]
    fn realm_send_round_trip() {
        let msg = NodeMessage::RealmSend {
            realm_id: 0x100,
            channel_id: 0xC0,
            author: 0xA,
            content: "안녕 hi".into(),
            nonce: Some("n-1".into()),
            reference_message_id: Some(0xBEEF),
        };
        let framed = msg.encode(3, 9);
        let (payload, _) = read_frame(&framed).unwrap().unwrap();
        let (_, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn realm_fanout_round_trip() {
        let msg = NodeMessage::RealmFanout {
            realm_id: 0x100,
            t: "GUILD_MEMBER_ADD".into(),
            payload: r#"{"realm_id":"256","user":{"id":"10"}}"#.into(),
            user_ids: vec![1, 2, 3, 99],
        };
        let framed = msg.encode(1, 0xABCD);
        let (payload, _) = read_frame(&framed).unwrap().unwrap();
        let (h, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(h.trace_id, 0xABCD);
        assert_eq!(decoded, msg);
    }

    #[test]
    fn realm_emit_round_trip() {
        let msg = NodeMessage::RealmEmit {
            realm_id: 0x100,
            t: "GUILD_MEMBER_REMOVE".into(),
            payload: r#"{"realm_id":"256","user":{"id":"10"}}"#.into(),
        };
        let framed = msg.encode(3, 0x55);
        let (payload, _) = read_frame(&framed).unwrap().unwrap();
        let (_, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn presence_gossip_round_trip() {
        let msg = NodeMessage::PresenceGossip { user_id: 0xABCDEF, node_id: 7, status: 1 };
        let framed = msg.encode(7, 0x1234);
        let (payload, _) = read_frame(&framed).unwrap().unwrap();
        let (h, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(h.msg_type, msg_type::PRESENCE_GOSSIP);
        assert_eq!(decoded, msg);
    }

    #[test]
    fn user_deliver_round_trip() {
        let msg = NodeMessage::UserDeliver {
            t: "RELATIONSHIP_ADD".into(),
            payload: r#"{"user":{"id":"10"},"kind":"pending_in"}"#.into(),
            user_ids: vec![10, 20, 30],
        };
        let framed = msg.encode(2, 0x77);
        let (payload, _) = read_frame(&framed).unwrap().unwrap();
        let (h, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(h.msg_type, msg_type::USER_DELIVER);
        assert_eq!(decoded, msg);
    }

    #[test]
    fn hello_round_trip_preserves_trace() {
        let msg = NodeMessage::Hello { capabilities: 0xDEAD, epoch: 12345 };
        let framed = msg.encode(1, 0xABCD);
        let (payload, _) = read_frame(&framed).unwrap().unwrap();
        let (h, decoded) = NodeMessage::decode(payload).unwrap();
        assert_eq!(h.trace_id, 0xABCD);
        assert_eq!(decoded, msg);
    }

    #[test]
    fn incomplete_frame_returns_none() {
        let framed = NodeMessage::Ping.encode(1, 0);
        assert!(read_frame(&framed[..2]).unwrap().is_none());
    }

    #[test]
    fn unknown_msg_type_errors() {
        let mut r = Reader::new(&[]);
        assert_eq!(
            NodeMessage::decode_body(0xFFFF, &mut r),
            Err(DecodeError::UnknownTag(0xFFFF))
        );
    }
}
