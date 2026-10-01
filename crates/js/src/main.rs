fn main() {
    let source = "function add(a, b) { return a + b; } let result = add(42, 10);";
    match js::parse(source) {
        Ok(program) => println!("Generated AST Tree: {program:#?}"),
        Err(error) => eprintln!("Parser Error: {error}"),
    }
}
