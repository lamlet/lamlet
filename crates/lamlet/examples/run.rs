fn main() {
    let path = std::env::args().nth(1).expect("file");
    let src = std::fs::read_to_string(&path).unwrap();
    match lamlet::run_source(&src) {
        Ok(vs) => for (i, v) in vs { println!("[{}] {:?}", i, v) },
        Err(e) => { eprintln!("{}", e); std::process::exit(1) }
    }
}
