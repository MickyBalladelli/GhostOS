# SynOS ABI

abi/synos-abi.toml is the only source for syscall operation numbers, RPC
frame layout, RPC methods, and boundary status values.

Generate the checked-in Rust and Swift bindings with:

    python3 tools/generate_abi.py

CI or a release build can verify generated files are current with
python3 tools/generate_abi.py --check. ABI version checks happen before kernel
dispatch or gateway service calls. An old or incompatible frame gets the stable
PROTOCOL_MISMATCH status and cannot mutate service state.
