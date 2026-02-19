fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = myr_core::domain_name();
    let _ = myr_adapters::adapter_name();
    myr_tui::run()?;
    Ok(())
}
