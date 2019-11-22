# Intro

`fzero` is a grammar-based fuzzer that generates a Rust application inspired
by the paper "Building Fast Fuzzers" by Rahul Gopinath and Andreas Zeller.
https://arxiv.org/pdf/1911.07707.pdf

You can find the F1 fuzzer here:

https://github.com/vrthra/F1

# Usage

Currently this only generates an application that does benchmarking, but with
some quick hacks you could easily get the input out and feed it to an
application.

## Example usage

```
D:\dev\fzero_fuzz>cargo run --release html.json test.rs test.exe 8
    Finished release [optimized] target(s) in 0.02s
     Running `target\release\fzero.exe html.json test.rs test.exe 8`
Loaded grammar json
Converted grammar to binary format
Optimized grammar
Generated Rust source file
Created Rust binary!

D:\dev\fzero_fuzz>test.exe
MiB/sec:    1773.3719
MiB/sec:    1763.8357
MiB/sec:    1756.8917
MiB/sec:    1757.1934
MiB/sec:    1758.9417
MiB/sec:    1758.9122
MiB/sec:    1758.7352
```

# Concept

This program takes in an input grammar specified by a JSON file. This JSON
grammar representation is converted to a binary-style grammar that is intended
for interpretation and optimization. A Rust application (source file) is
produced by the shape of the input grammar. This then is compiled using `rustc`
to an application for the local machine.

This doesn't have any constraints on the random number generation as it uses an
infinite supply of random numbers. There is no limitation on the output size
and the buffer will dynamically grow as the input is created.

# Unsafe code

This project uses a small amount of `unsafe` code to provide the same semantics
of `extend_from_slice` but in a much faster way (over 4x faster). Not quite
sure why it's much faster, but if you are uncomfortable with `unsafe` code,
feel free to set `SAFE_ONLY` to `true` at the top of `src/lib.rs`. This will
restrict this fuzzer to only generate safe code. I don't think this is
necessary but who knows :)

# Performance

The performance of this tool is separated into multiple categories. One is the
code generation side, how long it takes for the JSON to be compiled into a Rust
application. The other is the code execution speeds, which is how fast the
produced application can generate inputs.

## Code Generation

Code generation vastly outperforms the "Building Fast Fuzzers" paper. For
example when generating the code based on the `html.json` grammar, the F1
fuzzer took over 25 minutes to produce the code. This fuzzer is capable of
producing a Rust application in under 10 seconds.

## Code execution

This project is on some performance metrics about 20-30% slower than the F1
fuzzer, but these scenarios are rare. However, in most situations we've been
about to out-perform F1 by about 30-50%, and in extreme cases (html.json
depth=8) we've observed over a 4x speedup.

# Differences from the F1 fuzzer

The F1 fuzzer mentions a technique that will resolve to the nearest terminal
tokens when stack depth is exceeded. We haven't implemented this technique but
I don't think it's a huge impact on the generated inputs. This is something I
will look into in the future.

Due to not using globals this can easily be scaled out to multiple threads as
all random state and input generation are done in a structure.

There is no use of assembly in this project, and thus it can produce
highly-performant fuzzers for any architecture or environment that Rust can
compile against (pretty much identical to LLVM's target list).

