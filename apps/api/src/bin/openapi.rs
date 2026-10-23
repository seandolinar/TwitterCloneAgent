fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&twitter_clone_api::openapi::document())
            .expect("OpenAPI serializes")
    );
}
