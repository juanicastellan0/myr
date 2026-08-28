use std::sync::Arc;

use myr_adapters::mysql::MysqlApplicationBackendFactory;
use myr_application::spawn_application;
use myr_core::profiles::FileProfilesStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profiles = FileProfilesStore::load_default()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let application = {
        let _runtime_guard = runtime.enter();
        spawn_application(Arc::new(MysqlApplicationBackendFactory), profiles)
    };
    myr_gui::run(application)?;
    Ok(())
}
