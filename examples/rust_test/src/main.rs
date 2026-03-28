extern "C" {
    fn hello_from_cpp();
}

fn main() {
    println!("Hello from Rust within Forge!");
    unsafe {
        hello_from_cpp();
    }
}
