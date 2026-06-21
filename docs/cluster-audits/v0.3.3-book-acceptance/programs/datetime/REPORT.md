# Book-Acceptance Report — slice: datetime

Book source (primary): `fundamentals/datetime.mdx`
Binary: `target/release/shape` @ strict-flip-collection-dispatch HEAD (prebuilt).
Determinism: all inputs are FIXED datetimes (parse / from_epoch / @"literal").
No `now()` / `utc()` / `to_local()` anywhere. Expected values derived from book
semantics and written BEFORE first run.

## Programs

### small.shape (~75 LOC, 33 assertions)
- VM:  ec=0, stdout = `ALL_CHECKS_PASSED`
- JIT: ec=0, stdout = `ALL_CHECKS_PASSED` (JIT emits a `[jit-fallback]`
  diagnostic to STDERR only; stdout unaffected)
- VM stdout == JIT stdout: BYTE-IDENTICAL
- Result: PASS

Covers: construction (`parse`, `from_epoch`, `@"..."` literal), component
access (year..second), day-info (`day_of_week`/`is_weekend`/`day_of_year`/
`week_of_year`), formatting (`format` specifiers, `iso8601`, `unix_timestamp`),
timezone (`to_timezone`/`to_utc`/`timezone`/`offset`), arithmetic
(`add_days`/`add_hours`/`add_months` clamp, `+ 3d`/`- 1w`, literal chain),
comparisons (`is_before`/`is_after`/`is_same_day`).

### large.shape (~497 LOC, 115 assertions)
Calendar & Scheduling Engine: business-day calculator, recurring-meeting
expansion (weekly standup + monthly clamp chain), multi-timezone meeting board,
timesheet aggregator, epoch round-trip ledger, ISO-week/quarter report.
- VM:  ec=0, stdout = `ALL_CHECKS_PASSED`
- JIT: ec=0, stdout = `ALL_CHECKS_PASSED` (JIT falls back to interpreter; see
  V2-FrameDescriptor note below)
- VM stdout == JIT stdout: BYTE-IDENTICAL
- Result: PASS (after fixing one author-error expected value; see below)

## Expected-value rationale (book citations)
- `from_epoch(1705314600000)` -> `2024-01-15T10:30:00+00:00` — book "From Epoch".
- `2024-01-06` is Saturday, dow=5, week_of_year=1 — book "Day Information".
  Verified full week: 2024-01-01 is Monday (dow=0); weekend = Sat(5)/Sun(6).
- `format` specifiers (`%Y %m %d %H %M %A %B %Z %z`) — book "Formatting" table.
- `iso8601`/`rfc2822`/`unix_timestamp(2024-06-15T14:30:45Z)=1718461845` — book
  "Standard Formats" / "Formatting for Display".
- NYC of 2024-06-15T12:00Z = 08:00 (EDT UTC-4); 09:00-04:00 -> Tokyo 22:00;
  Tokyo of 12:00Z = 21:00 (UTC+9); London 13:00 (BST) — book "Timezone".
- `timezone()`="UTC+5:30", `offset()`="+05:30" — book "Inspecting Timezone Info".
- `add_months(Jan31)`->Feb29 (2024 leap), `add_months` again -> Mar29 — book
  "Adding Time". Nov30 +3mo -> 2025-02-28 (year-boundary clamp; verified probe).
- `@"2024-06-15" + 30d - 1w` = 2024-07-08 — book "DateTime Literals".
- `a + 3d` day=18, `a - 1w` day=8 — book "Operator Arithmetic".
- Weekdays in Jan 2024 inclusive = 23 (31 days − 8 weekend days) — book
  "Date Range Iteration" semantics.
- DateTime − DateTime -> Duration; prints ISO-8601 `PT432000S` (5 days). Book
  comment "5 days as a duration" is descriptive; no exact-string claim, so the
  `PT…S` form is consistent with the book (not asserted against a literal "5 days").

## Failure classifications

### 1. AUTHOR-ERROR (fixed, self-corrected as a real user would)
- `add_minutes(45)` on a 12:00:**00** base -> minute = 45, not 15. My first
  expected value (15) was a miscalculation. Fixed to 45; re-ran -> PASS.

### 2. FN-REG-CORRECTNESS (language defect — real-user-blocking)
**`int`-returning DateTime methods called on a `DateTime`-typed function
PARAMETER do not propagate their `int` return type into arithmetic / `let`
contexts.** The operands come back `unknown` and strict typing rejects the
binary op. First-run truth:

```
fn days_between(a: DateTime, b: DateTime) -> int {
    let secs = b.unix_timestamp() - a.unix_timestamp()   // ERROR
    secs / 86400
}
```
-> `error[SEMANTIC]: Cannot infer types for binary operation `Sub`: operand
types are `unknown` and `unknown`.`

Characterization (isolated probes):
- TOP-LEVEL receiver (`let d = DateTime.parse(...); let x = d.year(); x + ...`)
  RESOLVES correctly (`int`). 
- DateTime-PARAMETER receiver: `bool`-returning methods (`is_before`,
  `is_weekend`, `is_weekday`, `is_same_day`) and `DateTime`-returning
  (`add_days`) RESOLVE; a scalar method in pure return position
  (`fn(d){ d.year() } -> int`, `fn(d){ d.format(..) } -> string`) RESOLVES.
- But a scalar `int`-returning method on a parameter whose result feeds an
  arithmetic operator or a `let` binding loses its type (`unknown`).

Book impact: the book's Method Reference declares `unix_timestamp()`/`year()`/
… as returning `int`; a user writing the obvious helper
`fn days_between(a: DateTime, b: DateTime)` (a duration utility — exactly the
kind the "Arithmetic" section motivates) hits a hard compile error. Worked
around in large.shape by extracting timestamps at the top-level call site
(where resolution works) and passing `int`s into the helper. The workaround is
documented inline; the defect itself is NOT hidden.

### 3. (Adjacent) collection-element constraint — DateTime not array-storable
`[dt]` literal and `arr.push(dt)` both fail for a `DateTime` value, even with an
`Array<DateTime>` annotation (`the type of the value pushed here is not
statically known`). `Array<int>` / `Array<string>` work. The engine stores
epoch-ms ints / formatted strings instead. This is the same strict-flip
collection-dispatch class under test; recorded as a book_gap (the book never
shows collecting DateTime objects — only formatted strings — so it does not
contradict the book, but a user would reasonably try it).

## book_wrong
- **"Date Range Iteration" example (datetime.mdx lines 360-375)** is presented
  as correct code but FAILS to compile verbatim:
  ```
  let mut weekdays = []
  ...
  weekdays = weekdays + [current.format("%Y-%m-%d")]
  ```
  -> `error[SEMANTIC]: empty array `weekdays` has an un-resolvable element type`
  AND `Cannot infer types for binary operation `Add`: operand types are
  `unknown` and `string[]``. Under strict typing the empty `[]` needs an
  `Array<string>` annotation. The book teaches the un-annotated form as
  idiomatic. (With `let mut weekdays: Array<string> = []` it works and yields 23.)

## book_gaps
- The book never states that strict typing requires an explicit `Array<T>`
  annotation for an array that starts empty (`[]`) and is grown — yet every
  growable-collection example needs it. (Discovered against the book's own
  Date Range Iteration snippet; cross-checked behavior, no external fallback
  needed.)
- The book does not document that a `DateTime` value cannot be stored as an
  array element under strict typing (no `Array<DateTime>` example exists), so a
  user collecting DateTimes (rather than formatted strings) is on their own.
- The book does not warn that scalar (`int`/`string`) DateTime methods called on
  a `DateTime`-typed parameter may fail to type-resolve in arithmetic — see
  FN-REG-CORRECTNESS above.
- `DateTime - DateTime` "in milliseconds" (book line 296) actually prints an
  ISO-8601 duration `PT<seconds>S` (`PT432000S`); the book does not document the
  Duration value's string form, only the descriptive "5 days" comment.

## JIT note (not a slice defect)
Both programs run identically under `--mode jit`; the JIT declines to compile
(typed-array opcodes lack a FrameDescriptor -> "V2 bytecode verification
failed" SURFACE, tracked v0.4 in `docs/cluster-audits/v0.3-r8w6-hashmap-key-kind-audit.md`)
and falls through to the interpreter. STDOUT is byte-identical to VM in both
programs.
