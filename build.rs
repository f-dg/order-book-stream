fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("order_book.proto")?;
    Ok(())
}
