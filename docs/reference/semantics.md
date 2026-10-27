# Kiln semantics

What a Kiln program does when it runs. [syntax.md](syntax.md) says what
the parser accepts and [types.md](types.md) what the checker allows; this
page covers values, evaluation and failure. Every rule here is checked by a
conformance program in `conformance/`, run through both backends.

## Values and references

Kiln has two kinds of values.

- **Plain values:** `Int`, `Float`, `Bool`, `String`, `Duration`,
  `Instant`, `Date`, enums, and optionals of these. Assigning or passing
  one gives the other name its own value. Strings never change: `t = t +
  "c"` makes a new string, and `s` keeps the old one.
- **Shared values:** structs, records, lists, maps and sets. Assigning or
  passing one shares it. A change through one name is seen through every
  other, as in Java, JavaScript or Python.

```kiln
let a = { x: 1 }
let b = a
b.x = 5
print(a.x)          // 5
```

`let` and `var` decide whether a **name** can be pointed at another value,
not whether the value can change: `let posts = []` can still be pushed to.
To change a value without touching the original, copy it first:
`Post(..post, title: "New")`, `{ ..record, x: 0 }`, `list.slice(0,
list.len())`. These copies are shallow.

A `const` is different: it is read-only all the way down. Nothing may
assign to it, to its fields or entries, or call a method on it that
changes it (E0207).

The memory is managed by a garbage collector. There is no way to free a
value, and no value is freed while the program can still reach it.

## Equality and order

`==` compares by value for every type that has it: two structs, records,
lists, maps or sets are equal when their contents are, whatever their
identity. Maps and sets compare without regard to order. Functions cannot
be compared. Map keys and set items use the same equality, which is why a
list or struct should not be changed while it is a key.

Maps and sets keep their keys in the order they were first added.

## Evaluation

- Expressions are evaluated left to right, and a call's arguments before
  the call, in the order they are written.
- `and` and `or` evaluate their right side only when they need it; `??`
  evaluates its right side only for `none`.
- A closure sees the variables it uses, not copies of them: a `var`
  changed after the closure was made is seen with its new value.
- `..base` in a copy is evaluated once, before the fields that are
  named.
- Default arguments and field defaults are evaluated each time they are
  used.

## Numbers

`Int` is 64-bit. Overflow in `+`, `-` and `*` panics instead of wrapping,
and so does dividing by zero. `/` on integers rounds toward zero (`-7 / 2`
is `-3`) and `%` takes the sign of the left side (`-7 % 2` is `-1`).
`Float` is a 64-bit IEEE number. Numbers never convert implicitly:
`n.to_float()` and `f.round()` say what they mean.

## Failure

There are two ways to fail, and they mean different things.

- **Errors** are expected failures: a missing file, a bad request, a
  database that is down. A function that can fail says so with `!` in its
  type, the caller must handle the error with `try`, `catch`, `??` or
  `match`, and the checker makes sure it does. An error that leaves `main`
  is printed and the program exits with code 1. In a server, an error that
  leaves a handler becomes a 500 response, and the server keeps running.
- **Panics** are bugs: an index out of range, overflow, division by zero,
  a failed `panic(...)`. A panic prints its message and the program exits
  with code 101. In a server, a panic in a handler ends that request with
  a 500 response; the other requests carry on.

## Concurrency

A server handles each request on a task of its own. Tasks share one core
and switch only where a program waits: for a socket, a database or a
timer. Programs do not create tasks themselves yet. Kiln does not promise
that changing a value shared between handlers is safe, because a later
scheduler will use several cores: keep shared values unchanged. See
[stdlib.md](stdlib.md#tasks).

## Strings

Strings are UTF-8 text, measured in characters: `"héllo".len()` is 5,
`slice(start, end)` counts characters, and `chars()` gives them as a
list.
