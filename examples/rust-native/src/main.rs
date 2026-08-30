use aws_lc_rs::digest;

fn main() {
    println!("math: {}", math::answer());
    println!("blake3: {}", blake3::hash(b"forge").to_hex());
    println!("sha256: {} bytes", digest::digest(&digest::SHA256, b"forge").as_ref().len());
}
