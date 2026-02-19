fn main() {
    let domain = myr_core::domain_name();
    let ui = myr_tui::ui_name();
    let adapter = myr_adapters::adapter_name();

    println!("{domain} | {ui} | {adapter}");
}
