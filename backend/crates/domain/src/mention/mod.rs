//! 멘션 파싱 (개념: mention). 순수 로직 — IO 무의존 (D39).
//!
//! Discord 관례 `<@123>` / `<@!123>`(닉네임 멘션)에서 유저 id를 뽑는다. 역할 멘션(`<@&id>`)은 Phase 4.
//! 중복 제거 + 등장 순서 유지. 존재하는 유저인지 검증은 저장 어댑터의 몫(여기선 순수 파싱).

use crate::id::{Snowflake, UserId};

/// content에서 유저 멘션 id를 추출 (중복 제거, 등장 순서).
pub fn parse_mentions(content: &str) -> Vec<UserId> {
    let mut out: Vec<UserId> = Vec::new();
    let bytes = content.as_bytes();
    let mut search = 0usize;
    // "<@"(ASCII) 기준 스캔 — 토큰의 모든 구성요소가 ASCII라 바이트 오프셋이 항상 char 경계.
    while let Some(rel) = content[search..].find("<@") {
        let mut j = search + rel + 2;
        if bytes.get(j) == Some(&b'!') {
            j += 1; // 닉네임 멘션 형태.
        }
        let digits_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > digits_start
            && bytes.get(j) == Some(&b'>')
            && let Ok(id) = content[digits_start..j].parse::<u64>()
        {
            let uid = UserId(Snowflake::from_raw(id));
            if !out.contains(&uid) {
                out.push(uid);
            }
        }
        // 다음 탐색 위치 — 최소 1바이트 전진(무한루프 방지).
        search = j.max(search + rel + 2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u64) -> UserId {
        UserId(Snowflake::from_raw(n))
    }

    #[test]
    fn parses_plain_and_nick_mentions() {
        assert_eq!(parse_mentions("hi <@10> and <@!20>!"), vec![uid(10), uid(20)]);
    }

    #[test]
    fn dedups_and_keeps_order() {
        assert_eq!(parse_mentions("<@5> <@3> <@5>"), vec![uid(5), uid(3)]);
    }

    #[test]
    fn ignores_malformed_and_role_mentions() {
        assert!(parse_mentions("no mentions here").is_empty());
        assert!(parse_mentions("<@> <@abc> <@&99> <@12").is_empty());
    }

    #[test]
    fn handles_unicode_around_tokens() {
        assert_eq!(parse_mentions("안녕 <@7> 반가워"), vec![uid(7)]);
    }
}
