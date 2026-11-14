# 2단계 — B+tree 페이지 레이아웃 (diff 05)

## 1. 16KB = `u64` 2048칸에 맞추기

| 페이지 | 구성 | 칸 수 |
|---|---|---|
| 리프 `LeafPage` | kind · count · prev · next + keys[1022] + values[1022] | 4 + 1022 + 1022 = 2048 |
| 내부 `InternalPage` | kind · count + keys[1022] + children[1023] + 여분 1 | 2 + 1022 + 1023 + 1 = 2048 |
| 메타 `MetaPage` | magic(`IDXBTREE`) · root · height · max_keys + 여백 | 4 + 2044 = 2048 |

```rust
pub const LEAF_CAPACITY: usize = (PAGE_SIZE / 8 - 4) / 2;         // 1022
pub const INTERNAL_CAPACITY: usize = (PAGE_SIZE / 8 - 2 - 1) / 2; // 1022 — 헤더 2 + 키 m + 자식 m+1 ≤ 2048
const _: () = assert!(size_of::<LeafPage>() == PAGE_SIZE);        // 3종 모두
```

- 내부 노드는 2 + m + (m+1) = 2048 → 2m = 2045 → m = 1022, 1칸이 남아 `_reserved`.
- 변이: 여분을 2칸으로 → size 단언 컴파일 오류. 리프 식 `-4`→`-3`은 `2045/2 = 1022`(정수 나눗셈)라 결과가 같은 **동치 변이**.

## 2. `NO_PAGE = 0` — 왜 0이 "없음"인가

- 명세 S4: 인덱스 하나 = 파일 하나, **페이지 0 = 메타**. 노드는 1번부터 → 0은 노드 번호로 나올 수 없다.
- 노드가 0번에 놓일 수 있다면 `leaf.next == 0`이 "다음 리프 = 0번"인지 "다음 없음"인지 모른다 → 리프 순회가 일찍 멈추거나, 메타 페이지를 리프로 읽어 쓰레기를 따라간다.

## 3. `kind` 칸 — 1단계 `enum`의 대체

- 메모리 버전은 `enum Node { Internal, Leaf }`가 종류를 알려 줬다. 디스크에서 읽은 페이지는 바이트일 뿐 → 첫 칸 `kind`(1 = 리프, 2 = 내부)가 그 역할.
- 자식 페이지를 fetch한 뒤 `kind`를 봐야 `children[]`로 더 내려갈지, `values[]`를 읽을지 정한다.
- 0은 종류 값으로 쓰지 않는다 → 0으로 찬 페이지·메타 페이지는 `kind_of`가 `None` → 잘못된 페이지 번호나 손상을 노드로 착각하지 않는다.

## 4. 진행

| diff | 게이트 |
|---|---|
| 05 | ✅ 1/2 pass (질문에 코드 인라인 — 하네스 개선 첫 적용) |
