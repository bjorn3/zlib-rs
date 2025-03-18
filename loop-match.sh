#!/bin/bash


dfa="-Cllvm-args=-enable-dfa-jump-thread"

loop_match="--features=loop-match"

cargo +stage1 build --release --example blogpost-uncompress
cp target/release/examples/blogpost-uncompress target/release/examples/uncompress-baseline

RUSTFLAGS="$dfa" cargo +stage1 build --release --example blogpost-uncompress
cp target/release/examples/blogpost-uncompress target/release/examples/uncompress-llvm-dfa

cargo +stage1 build --release --example blogpost-uncompress $loop_match
cp target/release/examples/blogpost-uncompress target/release/examples/uncompress-loop-match

RUSTFLAGS="$dfa" cargo +stage1 build --release --example blogpost-uncompress $loop_match
cp target/release/examples/blogpost-uncompress target/release/examples/uncompress-llvm-dfa-loop-match

poop "target/release/examples/uncompress-baseline rs-chunked 4" 
    "target/release/examples/uncompress-llvm-dfa rs-chunked 4" 
    "target/release/examples/uncompress-loop-match rs-chunked 4" 
    "target/release/examples/uncompress-llvm-dfa-loop-match rs-chunked 4" 
