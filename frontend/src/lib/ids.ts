// Snowflake id는 backend에서 **JSON 문자열**로 직렬화된다(rest.md §0: JS 53비트 정수 절단 회피).
// 따라서 프론트 전 계층에서 id는 절대 number로 변환하지 않고 string으로만 다룬다.
export type Snowflake = string;

// Snowflake 상위 비트가 타임스탬프(밀리초, Discord epoch 유사)지만, backend epoch를 모르면
// 정렬에만 쓰는 게 안전하다. 큰 수 비교는 길이→사전식으로(동일 자릿수 가정 안전치 않으니 BigInt).
export function compareSnowflake(a: Snowflake, b: Snowflake): number {
  const x = BigInt(a);
  const y = BigInt(b);
  return x < y ? -1 : x > y ? 1 : 0;
}
