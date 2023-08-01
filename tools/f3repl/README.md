# f3repl

f3repl is a host tool that provides a REPL of the forth3 VM. It is used for testing and development of forth3.

When run, it provides an interactive repl.

```
cargo run --release
    Finished release [optimized] target(s) in 0.08s
     Running `f3repl`
> : star 42 emit ;
ok.
> star star star
***ok.
```
