# repCXL

CXL disaggregated memory experiments. swystems &lt;> IBM research. 

This repository contains the Rust repCXL library providing replication for 
disaggregated CXL memory nodes and a number of binaries to test its 
functionality. 

- `src/` RepCXL source
- `src/bin/` executable programs
- `tests/` RepCXL integration tests to be run with `cargo test`
- `config/` RepCXL configuration files
- `ansible/` automated benchmark workflows which collect results in `bench-outputs/`
- `jupyter/` test analysis and visualization with Jupyter Notebooks 
- `ycsb/` YCSB benchmark workflows and traces


## Pre-requisites

- Rust
- `ansible/README.md` for benchmark requirements.
