-- V19: 신규 월 메시지 파티션 사전 생성 (D28, 04 §6). "운영 작업"을 멱등 함수로.
--
-- V17은 2026_06/_07 + DEFAULT만 만들었다. 시간이 흐르면 새 달 메시지가 DEFAULT로 쌓여
-- "최근=핫" 지역성(04 §2)이 무너진다 → 다가오는 달 파티션을 미리 만들어 둬야 한다.
-- 달력 계산(월 경계)은 Postgres에 맡긴다(앱에 날짜 라이브러리 무도입). 앱 startup이 호출.
--
-- 파티션 경계 id = (month_start_ms - EPOCH_MS) << 22   (22 = worker10 + seq12, D11/04 §2)
--   EPOCH_MS = 1_700_000_000_000 (domain::id::EPOCH_MS). month_start = UTC 그 달 1일 00:00:00.
-- CREATE 는 to_regclass 가드로 멱등 (이미 있으면 스킵). 미래 달은 DEFAULT에 행이 없어 안전.

CREATE OR REPLACE FUNCTION ensure_message_partitions(months_ahead int DEFAULT 2)
RETURNS int
LANGUAGE plpgsql
AS $$
DECLARE
    epoch_ms  bigint := 1700000000000;            -- domain EPOCH_MS (D11)
    base      timestamp := date_trunc('month', now() AT TIME ZONE 'UTC');  -- 이번 달 1일 UTC 벽시계
    m         int;
    lo_tstz   timestamptz;
    hi_tstz   timestamptz;
    lo        bigint;
    hi        bigint;
    pname     text;
    created   int := 0;
BEGIN
    FOR m IN 0..months_ahead LOOP
        -- 월 가산은 tz 무관한 timestamp(벽시계)로 한 뒤 UTC로 못박아 timestamptz 화.
        lo_tstz := (base + make_interval(months => m))     AT TIME ZONE 'UTC';
        hi_tstz := (base + make_interval(months => m + 1)) AT TIME ZONE 'UTC';
        lo := ((floor(extract(epoch from lo_tstz) * 1000)::bigint - epoch_ms) << 22);
        hi := ((floor(extract(epoch from hi_tstz) * 1000)::bigint - epoch_ms) << 22);
        pname := 'messages_' || to_char(lo_tstz, 'YYYY_MM');
        IF to_regclass(pname) IS NULL THEN
            EXECUTE format(
                'CREATE TABLE %I PARTITION OF messages FOR VALUES FROM (%s) TO (%s)',
                pname, lo, hi);
            created := created + 1;
        END IF;
    END LOOP;
    RETURN created;
END;
$$;
