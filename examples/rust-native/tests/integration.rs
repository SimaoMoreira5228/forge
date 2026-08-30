#[test]
fn public_api_answers() {
    assert_eq!(math::answer(), 42);
}

#[test]
fn handles_edges() {
    assert_eq!(math::add(-2, 2), 0);
}
