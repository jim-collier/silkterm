# Fuzz corpus

One directory per fuzz target, named the way the target names itself. Every file in a directory is replayed on each run before the generated cases, and is also used as a parent for the mutator.

Two kinds of file belong here.

- Seeds: a realistic input, so the mutator has something true to chew on. Most of what is here now is a seed.

- A case that once broke something. Save it under a name that says what it was, and it is replayed forever afterwards. A defect that also has a plain unit test does not need a file here as well; this is for the ones only the fuzzer can express.

The engine and the targets are described at the top of `source/src/fuzz.rs`. The targets themselves sit beside the code they hammer, in a `mod fuzz` inside that module's tests.

To reproduce one case, take the seed from the failure and run that alone:

```sh
SILK_FUZZ_SEED=1234 cargo test <target name>
```

`SILK_FUZZ_SECS` sets how long each target gets. A plain `cargo test` leaves it unset and every target takes a fraction of a second; the pipeline sets it for a real soak.
