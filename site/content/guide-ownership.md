---
title: "Guide: Ownership and borrowing"
description: A long, friendly guide to how Drachm frees memory without a garbage collector, what moves, copies and borrows mean, and how to read and fix the compiler's errors.
---

# Ownership and borrowing

This guide is for people who have written Python, JavaScript, Java or C, and
have never used a language with an ownership system. It assumes nothing beyond
the [first nine lessons](lesson-1-first-program.md). Read it top to bottom once;
afterwards the [reference pages](reference-ownership.md) are the place to look
things up.

Every program and every error message on this page comes from the real
compiler. The programs that run have a block labelled "output"; the ones that
fail show what `drachm check` prints.

## 1. The problem: who frees the memory?

A program needs memory for things whose size is not known in advance: the text
of a string, the items of a list. That memory has to be handed back when the
program is done with it, or the computer runs out. Someone has to decide
when "done" is. Languages answer this in three ways.

**You do it by hand (C).** You ask for memory with `malloc` and give it back with
`free`. Nothing checks that you did it right, and three mistakes are easy to make:

```c
char *name = malloc(5);          // 1. ask for memory
strcpy(name, "Illy");
char *alias = name;              // 2. two names for one piece of memory
free(name);                      // 3. give it back ...
free(alias);                     //    ... and again: a DOUBLE FREE
printf("%s\n", name);            //    reading freed memory: USE AFTER FREE
```

Leave out the `free` and you have a **leak**: the memory is never returned, and
a long-running program slowly eats the machine. Use after free and double free
are worse. They corrupt memory, and the program may crash far from the bug, or
carry on and do something an attacker chose. A large share of security holes in
C and C++ programs are exactly these mistakes.

**A garbage collector does it for you (Python, JavaScript, Java).** The runtime
tracks which values can still be reached and frees the rest. This is safe and
easy, and the price is paid while the program runs: the collector needs CPU time
and extra memory, it may pause your program at moments you do not choose, and
you cannot say exactly when a value is destroyed (which makes it awkward to
close a file or release a lock "right when I am done").

**The compiler does it for you (Drachm).** Drachm has no garbage collector and no
`free`. Instead the compiler follows two ideas:

1. Every value has exactly **one owner**, and the value is destroyed when its
   owner goes out of scope.
2. Anything else that wants to use the value must **borrow** it, and the compiler
   checks that no borrow can outlive the value or get in the way of a change.

The compiler proves this by reading your program, before it runs. The finished
program contains the `free` calls in the right places, exactly as if you had
written them correctly in C, and nothing is checked while it runs. Double free
and use after free cannot be written in ordinary code, and an owned value is never
forgotten. A program that tries is rejected with an error that points at the line.

That is the whole trade. You learn a few rules and the compiler sometimes says
no; in return you get C's speed with a garbage collector's safety. The rest of
this guide is those rules, one idea at a time.

## 2. Ownership

### One owner, and scope ends

A `string` owns its text. When the name that owns it goes out of scope (reaches
the `}` of the block it was declared in), the text is freed. A struct can run
its own code at that moment with a method called `drop`, which makes the moment
visible:

<!-- example: guide-own-scope -->

<!-- focus: 12-13 -->
`a` is created and owns a `Guard`.

<!-- focus: 14-17 -->
`b` lives only inside the `if`. At its closing brace `b` is dropped, so "drop b"
prints before "end of main".

<!-- focus: 18-19 -->
At the end of `main` the remaining values are dropped in the reverse order they were
made: `c` first, then `a`. Reverse order means something made later, which may
depend on something made earlier, goes away first.

You never write the cleanup. It happens at the `}`, every time, even when the
function leaves early with `return` or `break`.

### Moving

If every value has one owner, what does `string b = a;` mean? It cannot make two
owners. It **moves** the value: `b` is now the owner and `a` has nothing.

```text title="what a move does"
string a = "Illy";           a ---> [ I l l y ]        (the text, in memory)

string b = a;                a  (empty)
                             b ---> [ I l l y ]        same text, new owner
```

Nothing is copied by a move. The text stays where it is; only the name that
owns it changes. That is why moves are cheap, and why passing a string to a
function, returning it, or storing it in a struct are all moves too.

Using `a` after the move would mean using a value it no longer owns, so the
compiler refuses:

```drachm
in io;

none main() {
    string a = "Illy";
    string b = a;
    io::outln(a);
}
```

```text title="drachm check"
error[E0301]: `a` is used after it moved
  --> main.drh:6:15
   |
 5 |     string b = a;
   |                - `a` moved here
 6 |     io::outln(a);
   |               ^ used here, after the move
   |
 help: to keep both, write `string b = copy a;`

main.drh: 1 error
```

The three parts to read in that message are the line where the value moved, the
line where you used it, and the `help` line, which offers the usual fix. With `copy`
the program works:

<!-- example: guide-own-fix-move -->

### Copying

The fix is `copy`. It makes a second, separate value with its own memory:

```text title="what copy does"
string b = copy a;           a ---> [ I l l y ]
                             b ---> [ I l l y ]        a second copy of the text
```

Copying a long string takes time and memory, so Drachm never does it behind
your back: **there are no hidden copies**. Wherever a copy happens, you wrote
`copy`, and you can search for it.

### Values that copy by themselves

Some values are so small that copying is as cheap as moving, and they would be
annoying to track. Numbers, bools, and structs made only of such things copy
automatically, with no `copy` needed. The exact rule:

- Numbers and bools always copy.
- A struct copies automatically when **all its fields copy** and its size in
  memory, **padding included, is at most 32 bytes**.
- Everything else moves: strings, structs holding strings, structs bigger than 32 bytes.

The size includes padding the compiler adds between fields, so rearranging
fields can change the size, and with it whether a struct is plain-copy.
A struct that must never copy by itself, however small, says `stays` after its name,
and then copying it is always explicit:

<!-- example: guide-own-copy -->

The compiler's errors line up with this. Moving the 40-byte `Big` and then
reading `big.a` is an error, and so is moving the `stays` struct `Handle`:

```drachm
struct Big { public: float64 a; float64 b; float64 c; float64 d; float64 e; }
struct Handle stays { public: uint32 id; }
none main() {
    Big big = 1.0, 2.0, 3.0, 4.0, 5.0;
    Big moved = big;
    float64 z = big.a;
    Handle h = 7;
    Handle h3 = h;
    uint32 k = h.id;
}
```

```text title="drachm check"
error[E0301]: `big.a` is used after `big` moved
  --> main.drh:6:17
   |
 5 |     Big moved = big;
   |                 --- `big` moved here
 6 |     float64 z = big.a;
   |                 ^^^^^ used here, after the move
   |
 help: to keep both, write `Big moved = copy big;`

error[E0301]: `h.id` is used after `h` moved
  --> main.drh:9:16
   |
 8 |     Handle h3 = h;
   |                 - `h` moved here
 9 |     uint32 k = h.id;
   |                ^^^^ used here, after the move
   |
 help: `Handle` is marked `stays`, so it never copies automatically; write `copy h`

main.drh: 2 errors
```

### Taking one field out

A struct owns its fields, so moving a field out leaves a hole in the struct.
Drachm makes you say so with `take`. A plain `string n = p.name;` is an error
(E0324) that suggests `take p.name` or `copy p.name`:

```drachm
struct Player {
public:
    string name;
    uint32 hp;
}

none main() {
    Player p = "Illy", 10;
    string n = p.name;
}
```

```text title="drachm check"
error[E0324]: cannot move `p.name` out of `p` without `take`
  --> main.drh:9:16
   |
 9 |     string n = p.name;
   |                ^^^^^^ this would leave `p` without a name
   |
 help: write `take p.name` to move it out, or `copy p.name` to keep both

main.drh: 1 error
```

`take p.name` moves the field out and leaves it **empty** until you give it a
new value; the compiler tracks that for you and rejects any read of an empty
field. `take p.title leaving "none"` moves the old value out and puts a new one in
during the same statement, so the field is never empty:

<!-- example: guide-own-take -->

Writing to a field needs permission too (a `mut` binding or a `mut` field; see
[section 5](#5-mutability)). A struct with a `drop()` method has to stay
complete so it can be dropped properly, so plain `take` on it is an error and only
`take x.f leaving v` is allowed. [Moves and copies](reference-ownership.md) has
the full rules, and [lesson 8](lesson-8-ownership.md) practises them.

## 3. Borrowing

### Looking and changing

Moving everything would be unbearable. A function that only wants to print a
string should not have to take it away from you. A **borrow** lets code use a
value without owning it:

- `&T` is a **shared** borrow: it may read.
- `&mut T` is a **unique** borrow: it may read and change.

The borrow is written at the call as well as in the parameter, so you can see in
`deposit(&mut acc, 20)` that `acc` may change. Inside the function a borrow works
like the value itself: reads are automatic (`a.balance`) and an assignment
writes through to the owner's value.

When a function that holds a `&mut` wants to pass it on and keep using it, it
writes `lend`. Passing it without `lend` would hand the borrow over (a `&mut`
moves, like a string) and the function could not use it again.

<!-- example: guide-own-borrow -->

A `&` borrow is just permission to look, so it copies freely; a `&mut` moves
unless you `lend` it. Borrowing needs no memory and no copying: it is a pointer
the compiler has checked.

### Many readers or one writer

Here is the rule the compiler enforces, and it is the heart of the system.
At any moment, a value has

- **any number of `&` borrows**, or
- **exactly one `&mut` borrow**,

and never both. People call it "shared XOR mutable". It sounds fussy, so here is
what it prevents.

**A reader whose data moves.** A string stores its text in a block of memory. When
it grows past the block, the text is copied to a bigger block and the old one is
freed. A `&string` that points at the old block would now read freed memory:

```text title="why you cannot change what is being read"
&string first = &name;       first ---> [ I l l y ]
                             name  ---> (same block)

name += "!";                 the block is full, so the text moves:

                             first ---> [ ??? ]      the old block is gone!
                             name  ---> [ I l l y ! ]
```

In C or C++ this is the classic "iterator invalidation" bug (holding a pointer
into a vector, then pushing to it). Drachm refuses to compile it, because
changing `name` while `first` will still be used breaks the rule:

```drachm
in io;

none main() {
    mut string name = "Illy";
    &string first = &name;
    name += "!";
    io::outln(first);
}
```

```text title="drachm check"
error[E0302]: cannot change `name` while it is borrowed
  --> main.drh:6:13
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
 6 |     name += "!";
   |             ^^^ change attempted here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before changing `name`

main.drh: 1 error
```

**Two writers.** If two names can both change one value, neither can rely on
what it sees. A function that checks `p.hp > 0` and then subtracts may find that
the other writer changed it in between. The compiler (and a reader of the code)
can assume that a `&mut` is alone, which also lets it generate faster code.
Two `&mut` at once is E0303, shown in [section 6](#6-how-the-compiler-checks-it-and-how-to-read-its-errors).

### Calls that look like they break the rule

A method call borrows its receiver, so `b::add(b::size())` seems to borrow `b`
twice, once to read and once to change. It is fine, because the borrow for the
change only starts when the call itself runs, and by then `b::size()` has
finished and handed back a plain number. This is called a **two-phase** borrow.
What is not fine is an argument that is still holding a borrow when the change
starts:

<!-- example: guide-own-two-phase -->

```drachm
in io;

struct Bag {
public:
    string a;
    &string first() { return &self.a; }
    none put(&string s) { self.a = copy s; }
}

none main() {
    mut Bag b = "x";
    b::put(b::first());
    io::outln(b.a);
}
```

```text title="drachm check"
error[E0302]: cannot change `b` while it is borrowed
   --> main.drh:12:12
    |
 12 |     b::put(b::first());
    |            ^ change attempted here
    |            - `b` is borrowed here
    |            ---------- the borrow is still used here
    |
 help: finish using the borrow before changing `b`

main.drh: 1 error
```

`b::first()` gives back a borrow of text inside `b`, and `put` is going to change
`b` while that borrow is still alive. The first three lines of the last program
are fine because `size()` returns a number, and `s::len()->text()` makes a new
string.

## 4. How long a borrow lives

### A borrow ends at its last use

A borrow does not last until the end of the block. It lasts until its **last
use**. The compiler looks at your program as a graph of what runs after what, and
asks, for every borrow, "is it used again from here?". If not, the borrow is
over and the value is free to change again.

<!-- example: guide-own-liveness -->

```text title="liveness of the first lines of that program"
&string first = &name;      first is born ----+
io::outln(first);           last use of first |   name is borrowed
name = "Sam";               -------------------+   name is free: changing is fine
```

This is why the programs you write usually need no extra braces or tricks to
satisfy the borrow rules. Hold a borrow only as long as you need it.
It also means the order of your lines matters, and many borrow errors are fixed
by moving the use up.

### Which parameter does the result come from?

A function can return a borrow, as `name_of` does in the [lesson](lesson-9-borrowing.md).
The caller must then keep the thing it points into safe, so the compiler needs to
know **which argument** the result borrows from. Most of the time it reads the body
and works that out itself. You write `lives` only when it is ambiguous.

<!-- example: guide-own-lives -->

Reading it top to bottom:

- `first_word` has one borrowed parameter, so the result can only come from `text`.
  Nothing to write.
- `longer` could return `a` or `b`, so it says `lives a, b`. The caller keeps both
  alive while it uses the result. Without that, the error is E0308, and its help
  line is the fix.
- `struct Reader lives src` has a borrowed field, so the struct says which field
  borrows. A `Reader` is only valid while the string it points at is.
- A method names `self` with `lives from`, joining sources with `or`:
  `lives from self or key`.

`lives` names whole parameters, never parts of them, and listing more than the
body needs is allowed (a caller just keeps more things alive). Here is the
error without it:

```drachm
&string longer(&string a, &string b) {
    if (a::len() > b::len()) { return a; }
    return b;
}

none main() { }
```

```text title="drachm check"
error[E0308]: `longer` returns a borrow, but which parameter it borrows from depends on the path
  --> main.drh:1:9
   |
 1 | &string longer(&string a, &string b) {
   |         ^^^^^^ the result borrows from `a` or `b`
 2 |     if (a::len() > b::len()) { return a; }
   |                                       - returns a borrow of `a` here
 3 |     return b;
   |            - returns a borrow of `b` here
   |
 help: write `lives a, b` after the parameters

main.drh: 1 error
```

Because `lives a, b` keeps *both* arguments borrowed for as long as the result
is used, changing either of them in that time is an error. That follows
directly from the rule, and it is pitfall 6 below.

See [Where a borrow comes from](reference-lives.md) for the other `lives`
errors (E0309, E0320) and for the limits of what it tracks.

## 5. Mutability

Ownership says who frees a value. **Mutability** says who may change it, and
Drachm makes you spell it out. By default a binding keeps the value it was given.
Three words change that:

| Word | On a | What it allows |
| --- | --- | --- |
| `mut` | binding | give the name a new value, and change its contents |
| `open` | binding | change the contents (fields, `+=`, lending `&mut`), never reassign |
| `mut` | field | change that field on a struct **you own**, even through a plain binding |

`open` also lets you declare a binding without a value and give it its one value
later (`open string xs;`), and write `open` parameters (`none fill(open string s)`).

<!-- example: guide-own-mut -->

Look at what the compiler says when you forget. A plain binding cannot be given
a new value, and cannot be lent as `&mut`:

```drachm
in io;

none main() {
    string s = "a";
    s = "b";
    io::outln(s);
}
```

```text title="drachm check"
error[E0205]: cannot change `s`: it is not `mut`
  --> main.drh:5:9
   |
 4 |     string s = "a";
   |            - declared here without `mut`
 5 |     s = "b";
   |         ^^^ assigned a second time here
   |
 help: declare it as `mut string s` to allow changes

main.drh: 1 error
```

```drachm
none grow(&mut string s) {
    s += "!";
}

none main() {
    string ys = "c";
    grow(&mut ys);
}
```

```text title="drachm check"
error[E0312]: cannot lend `ys` as `&mut`: it is neither `mut` nor `open`
  --> main.drh:7:10
   |
 6 |     string ys = "c";
   |            -- declared here without `mut` or `open`
 7 |     grow(&mut ys);
   |          ^^^^^^^ `&mut` borrow here
   |
 help: declare it `open string ys` (its contents may change) or `mut string ys` (it may also be reassigned)

main.drh: 1 error
```

### Methods and `mut` fields

You do not mark methods as changing or not. The compiler reads the body: a
method **changes `self`** if it writes to `self` or calls a method that does. That
gives three kinds of method:

- **reads only.** Works on any value, through `&` or `&mut`.
- **changes only `mut` fields** (like `hit` above). Works on a value you own
  even when the binding is not `mut`; not through a `&`; fine through `&mut`.
- **changes anything else.** Needs a `mut` or `open` binding (E0312) or a `&mut`.

A `mut` field is a convenience for things like counters on values you own. It is
not a way around the borrow rules: through a `&` borrow **nothing** changes, not
even a `mut` field. If you want to change something you need to hold `&mut`,
because code that only looks cannot also change.

```drachm
struct Player {
public:
    string title;
    mut uint32 hp;
}

none hurt(&Player p) {
    p.hp = p.hp - 1;
}

none main() { }
```

```text title="drachm check"
error[E0311]: cannot change `p.hp` through a `&` borrow
  --> main.drh:8:5
   |
 7 | none hurt(&Player p) {
   |           --------- `p` is a `&` borrow, which only reads
 8 |     p.hp = p.hp - 1;
   |     ^^^^ change attempted here
   |
 help: take a `&mut` borrow: declare it `&mut Player p`

main.drh: 1 error
```

There is no method that consumes `self`: methods only borrow it. To hand back a
field of a borrowed `self`, return a copy (`return copy self.name;`), because
the compiler will not guess that you meant one.

## 6. How the compiler checks it, and how to read its errors

### What happens to your program

You do not need this to use the language, but it helps explain the messages.
After type checking, the compiler turns each function into **MIR**, a small flat
program of numbered steps with the control flow made explicit. Then two passes
read it:

1. **Moves analysis** follows every path through the function and records, at
   each step, which names still own a value. This is how it knows `a` is empty
   after `string b = a;`, even if the move happened in only one branch of an
   `if` or in the previous trip round a loop, and where to put the drops.
2. **The borrow checker** records every borrow as a *loan* ("`&name` made at step 3")
   and works out the steps where it is still needed. Then it checks every step
   that moves, drops or changes something against the loans in force there.

`drachm check --dump-mir file.drh` prints the MIR of the E0302 program below.
Here is its heart (the `_2 = &_1` line is the borrow, and `Drop(_1)` followed by
`_1 = const "Sam"` is the change):

```text title="drachm check --dump-mir"
_1 = const "Illy"            name = "Illy"
_2 = &_1                     first = &name         <- the loan starts
Drop(_1)                     name is overwritten: the old text is dropped ...
_1 = const "Sam"             ... and replaced
_3 = &(*_2)                  outln(first) reads through first   <- the loan is used here
```

The `Drop` happens while the loan is still in use, and that is the error. The
same check runs the same way for every program, so the messages are consistent.
`--dump-borrowck` shows the loans and the steps they cover.

### Reading an error

Every borrow error has the same four parts. Here is E0302:

```drachm
in io;

none main() {
    mut string name = "Illy";
    &string first = &name;
    name = "Sam";
    io::outln(first);
}
```

```text title="drachm check"
error[E0302]: cannot change `name` while it is borrowed
  --> main.drh:6:5
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
 6 |     name = "Sam";
   |     ^^^^ change attempted here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before changing `name`

main.drh: 1 error
```

1. **The code and headline.** `E0302: cannot change name while it is borrowed`.
   `drachm explain E0302` says more, and the [error index](errors.md) lists every code.
2. **The location**, `main.drh:6:5`: file, line, column of the `^^^^` mark.
3. **The labelled lines.** Each marked line plays a role: the borrow was *made*
   here, the change is *attempted* here, and the borrow is *still used* here. The
   error is the sandwich: the first and last bracket a stretch of the program in
   which the middle thing is not allowed.
4. **`help`.** Usually the fix, written for this program.

Read the labels in line order and you can usually tell the story: "I borrowed
`name`, then I changed it, then I tried to use the borrow".

### E0301 to E0307

The moves and the borrows each have their own code. Here they are, in order, each
with the smallest program that makes it. E0301 you have already met above
([E0301 in section 2](#moving)): a value is used after it moved. It is also the error when a
value moves inside a loop and the next trip round uses it, or when it moved on
only some paths.

**E0302: change while borrowed.** Shown above. The fix is to finish with the borrow
first, or to borrow again later.

**E0303: two `&mut` at once.** A second `&mut` to `p` is made while the first is
still going to be used:

```drachm
in io;

struct Player {
public:
    mut uint32 hp;
}

none heal(&mut Player p) {
    p.hp = p.hp + 1;
}

none main() {
    mut Player p = 1;
    &mut Player a = &mut p;
    &mut Player b = &mut p;
    heal(a);
    heal(b);
}
```

```text title="drachm check"
error[E0303]: `p` is already borrowed with `&mut`
   --> main.drh:15:21
    |
 14 |     &mut Player a = &mut p;
    |                     ------ first `&mut` borrow here
 15 |     &mut Player b = &mut p;
    |                     ^^^^^^ second `&mut` borrow here
 16 |     heal(a);
    |          - the first borrow is still used here
    |
 help: only one `&mut` borrow of `p` can be in use at a time; finish using `a` before borrowing `p` again

main.drh: 1 error
```

Use the first borrow up before making the second, or use one borrow for both calls.

**E0304: using a value while it is `&mut` borrowed.** While a unique borrow is
going to be used, even *reading* the owner is unsafe, because the borrow may be
mid-change:

```drachm
in io;

struct Player {
public:
    mut uint32 hp;
}

none heal(&mut Player p) {
    p.hp = p.hp + 1;
}

none main() {
    mut Player p = 1;
    &mut Player a = &mut p;
    io::outln(p.hp::text());
    heal(a);
}
```

```text title="drachm check"
error[E0304]: cannot use `p.hp` while `p` is borrowed with `&mut`
   --> main.drh:15:15
    |
 14 |     &mut Player a = &mut p;
    |                     ------ `p` is borrowed with `&mut` here
 15 |     io::outln(p.hp::text());
    |               ^^^^ used here
 16 |     heal(a);
    |          - the `&mut` borrow is still used here
    |
 help: read `p.hp` before borrowing `p` with `&mut`, or read it through `a`: `a.hp`

main.drh: 1 error
```

Read through the borrow, as the help line says. This one runs:

<!-- example: guide-own-fix-reading -->

**E0305: moving a value that is borrowed.** The borrower would be left pointing at
a value that now lives somewhere else:

```drachm
in io;

none consume(string s) {
    io::outln(s);
}

none main() {
    string name = "Illy";
    &string first = &name;
    consume(name);
    io::outln(first);
}
```

```text title="drachm check"
error[E0305]: cannot move `name` while it is borrowed
   --> main.drh:10:13
    |
  9 |     &string first = &name;
    |                     ----- `name` is borrowed here
 10 |     consume(name);
    |             ^^^^ moved here
 11 |     io::outln(first);
    |               ----- the borrow is still used here
    |
 help: finish using `first` before moving `name`, or move a copy: `copy name`

main.drh: 1 error
```

Finish with `first` before the move, or hand over a copy.

**E0306: a borrow that outlives its value.** `local` is dropped at the end of its
block, but `keep` is used after it:

```drachm
in io;

none main() {
    mut &string keep = &"a";
    if (true) {
        string local = "x";
        keep = &local;
    }
    io::outln(keep);
}
```

```text title="drachm check"
error[E0306]: `local` does not live long enough
  --> main.drh:8:5
   |
 7 |         keep = &local;
   |                ------ `local` is borrowed here
 8 |     }
   |     ^ `local` is dropped here, at the end of its block
 9 |     io::outln(keep);
   |               ---- the borrow is still used here
   |
 help: declare `local` outside the block, so it lives as long as `keep` is used

main.drh: 1 error
```

The help line is the fix: make the value live as long as the borrow. This is "use
after free", caught at compile time:

<!-- example: guide-own-fix-block -->

**E0307: returning a borrow of a local.** A function's own values die when it
returns, so a borrow of one would dangle:

```drachm
&string shout() {
    string s = "hi"::upper();
    return &s;
}

none main() { }
```

```text title="drachm check"
error[E0307]: cannot return a borrow of `s`, which belongs to this function
  --> main.drh:3:12
   |
 2 |     string s = "hi"::upper();
   |            - `s` is created here and dropped when `shout` returns
 3 |     return &s;
   |            ^^ the borrow is returned here
   |
 note: `shout` takes no borrows, so a borrow it returns can only point at something it owns
 help: return the string itself: change the return type to `string` and write `return s;`

main.drh: 1 error
```

Return the string itself (`return s;`), which moves it to the caller:

<!-- example: guide-own-fix-return -->

## 7. Common pitfalls and their fixes

Each pitfall is a short program that fails, the real error, and a version that
works.

### 1. Using a value after it moved into a call

Passing `name` to `greet(string s)` hands the text over. The second call has
nothing left:

```drachm
in io;

none greet(string s) {
    io::outln("Hello, " + s);
}

none main() {
    string name = "Illy";
    greet(name);
    greet(name);
}
```

```text title="drachm check"
error[E0301]: `name` is used after it moved
   --> main.drh:10:11
    |
  9 |     greet(name);
    |           ---- `name` moved here
 10 |     greet(name);
    |           ^^^^ used here, after the move
    |
 help: if `greet` should not take ownership, give it a `&string` parameter; or pass a copy: `greet(copy name)`

main.drh: 1 error
```

If `greet` only needs to look, make the parameter a borrow. This is nearly always
better than `copy`:

<!-- example: guide-own-fix-greet -->

Use `greet(copy name)` when the function really needs its own string, for example
to store it.

### 2. Moving a field out of a struct

You met this in [section 2](#taking-one-field-out): `string n = p.name;` is
E0324. The fixes are `copy p.name` if both places need the text, or
`take p.name` (with `leaving` to keep the struct tidy) if the struct no longer
does.

<!-- example: guide-own-fix-field -->

### 3. Returning a borrow of something local

A parameter taken **by value** is a local variable of the function. Returning a
borrow of it is E0307 too:

```drachm
in io;

struct Player {
public:
    string name;
    uint32 hp;
}

&string name_of(Player p) {
    return &p.name;
}

none main() {
    Player p = "Illy", 10;
    io::outln(name_of(p));
}
```

```text title="drachm check"
error[E0307]: cannot return a borrow of `p`, which belongs to this function
   --> main.drh:10:12
    |
  9 | &string name_of(Player p) {
    |                 -------- `p` is taken by value, so it is dropped when `name_of` returns
 10 |     return &p.name;
    |            ^^^^^^^ the borrow is returned here
    |
 note: `name_of` takes no borrows, so a borrow it returns can only point at something it owns
 help: take it as a borrow: `&Player p`, or return `p` itself

main.drh: 1 error
```

Take the parameter as a borrow, so the caller keeps owning it and the function can
hand back a borrow into the caller's value:

<!-- example: guide-own-fix-name -->

Or return an owned value (`return copy p.name;`) and change the return type to `string`.

### 4. Changing something while it is borrowed

Two shapes of this are worth knowing. Growing a string with `+=` counts as a
change, exactly like assigning to it (you saw it in [section 3](#many-readers-or-one-writer)). The same holds for a field: `n.text` is borrowed, so `n.text += "!"` is a change.

```drachm
in io;

struct Note {
public:
    mut string text;
}

none main() {
    mut Note n = "ab";
    &string r = &n.text;
    n.text += "!";
    io::outln(r);
}
```

```text title="drachm check"
error[E0302]: cannot change `n.text` while it is borrowed
   --> main.drh:11:15
    |
 10 |     &string r = &n.text;
    |                 ------- `n.text` is borrowed here
 11 |     n.text += "!";
    |               ^^^ change attempted here
 12 |     io::outln(r);
    |               - the borrow is still used here
    |
 help: finish using `r` before changing `n.text`

main.drh: 1 error
```

The fix is to finish with `first` before the change, or to not hold the borrow:

<!-- example: guide-own-fix-change -->

The other shape is the two-phase one from [section 3](#calls-that-look-like-they-break-the-rule):
`b::put(b::first())` changes `b` while the argument still borrows it. Pass a
number or a copy instead of a borrow of the same value.

### 5. Borrowing in a loop

A borrow made before a loop and used inside it is "still going to be used" on
every trip round, so a change inside the loop breaks it, even though the use comes
*before* the change in the text:

```drachm
in io;

none main() {
    mut string name = "a";
    &string first = &name;
    mut uint32 i = 0;
    while (i < 2) {
        io::outln(first);
        name = "b";
        i += 1;
    }
}
```

```text title="drachm check"
error[E0302]: cannot change `name` while it is borrowed
  --> main.drh:9:9
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
  ...
 8 |         io::outln(first);
   |                   ----- the borrow is used again here, on the next time round the loop
 9 |         name = "b";
   |         ^^^^ change attempted here
   |
 help: finish using `first` inside the loop, or borrow `name` again each time round

main.drh: 1 error
```

Make the borrow inside the loop, so each trip round has a new one that ends before
the change:

<!-- example: guide-own-fix-loop -->

### 6. Holding a borrow longer than you meant

A function that returns a borrow ties the result to its arguments. With
`lives a, b` the result `r` keeps both `x` and `y` borrowed for as long as `r`
is used:

```drachm
in io;

&string longer(&string a, &string b) lives a, b {
    if (a::len() > b::len()) { return a; }
    return b;
}

none main() {
    string x = "long one";
    mut string y = "short";
    &string r = longer(&x, &y);
    y = "changed";
    io::outln(r);
}
```

```text title="drachm check"
error[E0302]: cannot change `y` while it is borrowed
   --> main.drh:12:5
    |
 11 |     &string r = longer(&x, &y);
    |                            -- `y` is borrowed here
 12 |     y = "changed";
    |     ^ change attempted here
 13 |     io::outln(r);
    |               - the borrow is still used here
    |
 help: finish using `r` before changing `y`

main.drh: 1 error
```

You are not using `y` in a clever way here; you are changing something that `r` may
still point into. Use `r` first:

<!-- example: guide-own-fix-lives -->

A struct is conservative in the same way. The compiler does not track which part
of a parameter a result points at, so after `&string r = rest(&p);` the whole of
`p` is borrowed while `r` is used:

```drachm
in io;

struct Parser lives src {
public:
    &string src;
    mut uint32 pos;
}

&string rest(&Parser p) {
    return p.src;
}

none main() {
    string text = "hello";
    mut Parser p = &text, 0;
    &string r = rest(&p);
    p.pos = 3;
    io::outln(r);
}
```

```text title="drachm check"
error[E0302]: cannot change `p.pos` while it is borrowed
   --> main.drh:17:5
    |
 16 |     &string r = rest(&p);
    |                      -- `p` is borrowed here
 17 |     p.pos = 3;
    |     ^^^^^ change attempted here
 18 |     io::outln(r);
    |               - the borrow is still used here
    |
 help: finish using `r` before changing `p.pos`

main.drh: 1 error
```

When that is too strict, finish with the borrow first, or copy what you need out
(`string r = copy p.src;`).

### 7. A `copy` that does nothing

`copy` is for values that would otherwise move. On a number it has nothing to do,
so the compiler warns (W0322). It is a warning, not an error, and your program
still builds:

```drachm
in io;

none main() {
    uint32 n = 5;
    uint32 m = copy n;
    io::outln(m::text());
}
```

```text title="drachm check"
warning[W0322]: `copy` does nothing for `uint32`
  --> main.drh:5:16
   |
 5 |     uint32 m = copy n;
   |                ^^^^ a copy happens anyway
   |
 help: remove `copy`: `uint32` always copies

main.drh: 1 warning
```

Remove the `copy`, as the help says (`uint32 m = n;`). The warning exists
because a stray `copy` makes a reader wonder what is being protected.
A setting to switch this lint off is part of the planned
[project config](design-config.md); it does not exist yet.

### 8. Passing `m` when the function wants `&m`

The call must show what the callee may do. Passing the value itself where a
borrow is expected is E0321, and the help line is the exact call to write:

```drachm
in io;

none shout(&mut string s) {
    s = s::upper();
}

none show(&string s) {
    io::outln(s);
}

none main() {
    mut string word = "quiet";
    shout(word);
    show(word);
}
```

```text title="drachm check"
error[E0321]: `shout` takes `&mut string` here, so the call must say so
   --> main.drh:13:11
    |
 13 |     shout(word);
    |           ^^^^ this passes `word` itself
    |
 help: write `shout(&mut word)`

error[E0321]: `show` takes `&string` here, so the call must say so
   --> main.drh:14:10
    |
 14 |     show(word);
    |          ^^^^ this passes `word` itself
    |
 help: write `show(&word)`

main.drh: 2 errors
```

Write `shout(&mut word)` and `show(&word)`. The `&mut` also needs `word` to be
`mut` or `open` (E0312). Two related mistakes:

- Passing a `&mut` you hold on to a second function **moves** it, so the next
  use is E0301. Write `lend s`:

```drachm
in io;

none grow(&mut string s) {
    s += "!";
}

none twice(&mut string s) {
    grow(s);
    grow(s);
}

none main() {
    mut string s = "hey";
    twice(&mut s);
    io::outln(s);
}
```

```text title="drachm check"
error[E0301]: `s` is used after it moved
  --> main.drh:9:10
   |
 8 |     grow(s);
   |          - `s` (a `&mut` borrow) moved here
 9 |     grow(s);
   |          ^ used here, after the move
   |
 help: to keep using `s` afterwards, pass it on with `lend s`

main.drh: 1 error
```

- Passing a `&mut` you hold where a `&` is wanted needs `&s`, which lends a
  shared view for the call. A bare `show(s)` is E0330:

```drachm
in io;

none show(&string s) {
    io::outln(s);
}

none twice(&mut string s) {
    show(s);
    show(&s);
}

none main() {
    mut string s = "hey";
    twice(&mut s);
}
```

```text title="drachm check"
error[E0330]: `s` is a `&mut string`, but a `&string` is wanted here
  --> main.drh:8:10
   |
 8 |     show(s);
   |          ^ passes the `&mut` itself
   |
 help: write `&s`: a shared borrow through the `&mut` you hold

main.drh: 1 error
```

Both fixes in one program that runs:

<!-- example: guide-own-fix-passing -->

### 9. A struct that holds a borrow, without `lives`

A field of type `&string` means the struct is only valid while something else
is. The struct has to say so:

```drachm
struct Holder {
public:
    &string text;
}

none main() { }
```

```text title="drachm check"
error[E0314]: `Holder` holds a borrow, so it must say what it borrows from
  --> main.drh:1:8
   |
 1 | struct Holder {
   |        ^^^^^^
 2 | public:
 3 |     &string text;
   |             ---- this field is a borrow
   |
 help: add `lives text`: `struct Holder lives text { ... }`

main.drh: 1 error
```

<!-- example: guide-own-fix-holder -->

If the struct could simply own its text, a `string` field avoids the whole
question. Prefer owning fields unless you are avoiding a big copy.

## 8. The escape hatch: `unsafe`

The checks are conservative. They only accept programs they can prove safe, so a
few correct programs get rejected, and a few jobs (talking to hardware, writing
your own data structure, calling C) cannot be proved at all. For those,
Drachm has raw pointers, and the compiler takes its hands off.

Using them needs two things, so nobody uses them by accident: the line
`//--!$unsafe: true` at the top of the file, and an `unsafe { }` block around
each operation on a pointer. `*T` is a pointer you can write through, and
`address of n` makes one from a `mut` or `open` binding. Making, reading and
writing through a pointer all go inside `unsafe { }`. `address of` an
*immutable* binding gives a `*fixed T`, which can only be read.

<!-- example: guide-own-unsafe -->

Inside the block you promise that the pointer is valid and that nothing else is
changing the value. Break that promise and you have C's bugs back: nothing checks
a raw pointer for use after free. Raw pointers also move like strings, and there is
no `*mut`: one pointer kind, `*T`.

You will rarely need any of this. Strings, structs, borrows and `lives` cover
ordinary programs, and the rules in this guide are what keep them safe. Keep
`unsafe` blocks small and wrap them in a function with a safe signature, so the
promise is made in one place. [Unsafe and pointers](reference-unsafe.md) has the
rest, including `freeze`, `thaw` and `unsafe` functions.

## 9. If you know Rust or C++

Drachm's ideas will look familiar if you have used Rust, and its goals overlap
with C++'s. The rules are close in places and different in others, and the
syntax is its own. A factual comparison:

| | Drachm | Rust | C++ |
| --- | --- | --- | --- |
| Assigning a string | moves | moves | copies |
| Which types copy by themselves | numbers, bools, structs of those up to 32 bytes; `stays` opts out | types that implement `Copy` | almost everything |
| Making a copy | `copy x`, always written | `x.clone()` | implicit, or written |
| Borrow at the call | `f(&x)`, `f(&mut x)` | `f(&x)`, `f(&mut x)` | `f(x)`; the reference is only in the signature |
| Passing a `&mut` on | `lend s` | implicit reborrow | not applicable |
| Taking a field out | `take p.f`, `take p.f leaving v` | partial move, or `mem::take` / `mem::replace` | `std::move(p.f)` |
| Who can change a value | `mut`, `open`, `mut` fields | `mut`, interior types (`Cell`, `RefCell`) | `const` and `mutable` |
| Does a method change `self` | inferred from the body | declared (`&self` / `&mut self`) | declared (`const`) |
| Method that consumes `self` | none: methods only borrow | yes (`self`) | no (moving is by hand) |
| Lifetimes | inferred; `lives` names whole parameters when ambiguous | `'a` parameters, with elision rules | none; the compiler does not check |
| How long a borrow lives | until its last use | until its last use | not checked |
| Cleanup | scope end; `drop()`; `destroy x;` to run it early | scope end; `Drop`; `drop(x)` | destructors |
| Opting out | `unsafe` plus a file flag; `*T`, `*fixed T` | `unsafe`; `*const T`, `*mut T` | everything is allowed |
| Runtime cost of the checks | none, all at compile time | none | none (nothing is checked) |

The differences that matter day to day: copies are always written and there are no
hidden ones; a field leaves a struct only through `take`; `lend` replaces silent
reborrowing so you can see where a `&mut` goes; methods need no annotation to say
whether they change `self`; and the keywords are English words rather than
sigils and apostrophes.

## Where to go next

- [Lesson 8: Ownership](lesson-8-ownership.md) and [Lesson 9: Borrowing](lesson-9-borrowing.md)
  have exercises on everything in sections 2 and 3.
- [Moves and copies](reference-ownership.md) states every rule, with each error.
- [Where a borrow comes from](reference-lives.md) covers `lives` in full.
- [Unsafe and pointers](reference-unsafe.md) covers raw pointers.
- The [error index](errors.md) lists every code, including the ones in this guide.
