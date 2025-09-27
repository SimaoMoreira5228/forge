fn main() {
	let data = b"Hello, blake3!";

	let hash = blake3::hash(data);

	println!("Data: {}", std::str::from_utf8(data).unwrap());
	println!("Blake3 hash: {}", hash);

	let mut hasher = blake3::Hasher::new();
	hasher.update(b"First chunk ");
	hasher.update(b"Second chunk");
	let streaming_hash = hasher.finalize();

	println!("Streaming hash: {}", streaming_hash);

	let combined_data = b"First chunk Second chunk";
	let direct_hash = blake3::hash(combined_data);

	println!("Hashes match: {}", streaming_hash == direct_hash);
}
