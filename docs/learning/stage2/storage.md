# 2단계 — 저장소 · Pager (diff 01)

## 1. 무엇을 만들었나

| 이름 | 역할 |
|---|---|
| `PAGE_SIZE = 16 * 1024` | 디스크 I/O·버퍼풀 프레임·인덱스 노드의 단위 |
| `PageId = u64` | 페이지 번호. 파일 위치 = `id × PAGE_SIZE` |
| `Page` (`#[repr(C, align(8))]`, `Pod`) | 페이지 한 장의 바이트. 8바이트 정렬이라 `u64` 배열 구조체로 그대로 캐스팅된다 |
| `Storage` trait | `page_count` · `read_page` · `write_page`(끝에 붙이기만, 구멍 금지) |
| `MemStorage` | 테스트용 메모리 저장소 — 파일 저장소(diff 02)와 같은 의미 |
| `Pager` | 번호 할당(`allocate` — I/O 없음) + **성공한** read/write만 센다(P1) |

## 2. A1 스모크 — 첫 빌드에서 가정이 깨졌다

- `#[derive(Pod, Zeroable)]`을 `[u8; 16384]`에 붙이자 `E0277: [u8; 16384]: Zeroable` 미구현.
- 이유: `bytemuck` 기본 설정은 배열 impl을 정해진 길이 목록(0~32, 64, … 4096 등)에만 제공한다. 임의 길이 `[T; N]`은 feature `min_const_generics`가 켜야 한다.
- 해소: `bytemuck = { version = "1.25.2", features = ["derive", "min_const_generics"] }`.
- 가드 변이로 확인: 헤더 `u64 → u32`면 "derive(Pod) was applied to a type with padding", 배열을 1칸 줄이면 `size_of == PAGE_SIZE` 단언이 **컴파일 오류**로 잡는다.

## 3. 정렬(align)이 왜 필요한가 — 실측

`align(8)`을 뺀 `Page`(1바이트 정렬)를 `[u64; 2048]`로 `try_cast_ref`:

| 페이지 시작 오프셋 | 결과 |
|---|---|
| 0 | 성공 |
| 1 | `Err(TargetAlignmentGreaterAndInputNotAligned)` (`cast_*`면 패닉) |
| 8 | 성공 |

→ **항상** 실패하는 게 아니라 주소 운에 따라 간헐적으로 터진다. 테스트가 우연히 통과하는 버그가 된다. `align(8)`은 그 운을 타입 수준에서 제거한다.

## 4. 진행

| diff | 내용 | 게이트 |
|---|---|---|
| N2 재확인 | 버킷에 `mix(key)`를 저장하면? | check 부분 힌트 없이 정답 · range는 보충 |
| 01 | 저장소 · Pager · A1 | ✅ 1/2 pass |

## 5. 오답 노트

- **N2 range**: "mix 순서로 정렬된다"만 봄 → 정렬보다 **먼저** 필터 `lo <= k && k <= hi`가 깨진다. `mix(30)`은 약 5×10^17인 숫자라 `30..=33`에 들 일이 거의 없어 **빈 Vec**. 우연히 통과해도 반환되는 `k`는 사용자 키가 아니라 mix 값.
- **N2 오개념**: "문자열로 비교" → 아니다. `mix`는 `u64 → u64`, 숫자끼리 크기 비교.
- **N2 check**: "mix 다시 안 해도 됨"은 정답. 단 "검사 자체가 필요 없어진다"는 과잉 — 저장값 하위 d비트 = 칸 접미사 비교(제자리 검사)는 그대로 남는다.
- **01 Q2**: "정렬이 안 맞아 패닉" → 주소 의존(위 §3). 늘 터지는 게 아니라서 더 위험하다.
