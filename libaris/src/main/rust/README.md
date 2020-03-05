# Aris

## Web-Client Build

1. Install `wasm-pack` using `cargo install wasm-pack`
2. Build using `wasm-pack build --target web -- --features=js`
3. Browse to `index.html` or serve with `python3 -m http.server`

## Auto-Grader Build

1. Build using `cargo build --release --bin auto_grader`
2. Use with `target/release/auto_grader <instructor assignment> <student assignment>`

## License

This project is licensed under the GNU GPLv3 License.