extern crate irc;
extern crate futures;
extern crate tokio_core;
extern crate huawei_modem;
#[macro_use] extern crate diesel;
extern crate r2d2;
extern crate r2d2_diesel;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate toml;
#[macro_use] extern crate failure;
#[macro_use] extern crate log;
extern crate log4rs;
extern crate tokio_timer;

mod config;
mod store;
mod modem;
mod comm;
mod util;
mod schema;
mod models;
mod contact;
mod contact_factory;
mod control;

use config::Config;
use store::Store;
use modem::ModemManager;
use control::ControlBot;
use comm::{ChannelMaker, InitParameters};
use futures::Future;
use contact_factory::ContactFactory;
use futures::sync::mpsc::UnboundedSender;
use comm::ControlBotCommand;
use tokio_core::reactor::Core;
use log4rs::config::{Appender, Logger, Root};
use log4rs::config::Config as LogConfig;
use log4rs::append::Append;
use log4rs::append::console::ConsoleAppender;
use log::Record;
use std::fmt;

pub struct IrcLogWriter {
    sender: UnboundedSender<ControlBotCommand>
}
impl fmt::Debug for IrcLogWriter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "IrcLogWriter {{ /* fields hidden */ }}")
    }
}
impl Append for IrcLogWriter {
    fn append(&self, rec: &Record) -> Result<(), Box<::std::error::Error + Sync + Send>> {
        self.sender.unbounded_send(
            ControlBotCommand::Log(format!("{}:{} -- {}", rec.target(), rec.level(), rec.args())))
            .unwrap();
        Ok(())
    }
    fn flush(&self) {
    }
}

fn main() -> Result<(), failure::Error> {
    eprintln!("[+] smsirc starting -- reading config file");
    let config_path = ::std::env::var("SMSIRC_CONFIG")
        .unwrap_or("config.toml".to_string());
    eprintln!("[+] config path: {} (set SMSIRC_CONFIG to change)", config_path);
    let config: Config = toml::from_str(&::std::fs::read_to_string(config_path)?)?;
    let stdout = ConsoleAppender::builder().build();
    let mut cm = ChannelMaker::new();
    let ilw = IrcLogWriter { sender: cm.cb_tx.clone() };
    eprintln!("[+] initialising better logging system");
    let cll = config.chan_loglevel.as_ref().map(|x| x as &str).unwrap_or("info").parse()?;
    let pll = config.stdout_loglevel.as_ref().map(|x| x as &str).unwrap_or("info").parse()?;
    let log_config = LogConfig::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .appender(Appender::builder().build("irc_chan", Box::new(ilw)))
        .logger(Logger::builder()
                .appender("irc_chan")
                .build("sms_irc", cll))
        .build(Root::builder().appender("stdout").build(pll))?;
    log4rs::init_config(log_config)?;
    info!("Logging initialized");
    info!("Connecting to database");
    let store = Store::new(&config)?;
    info!("Initializing tokio");
    let mut core = Core::new()?;
    let hdl = core.handle();
    info!("Initializing modem");
    let mm = core.run(ModemManager::new(InitParameters {
        cfg: &config,
        store: store.clone(),
        cm: &mut cm,
        hdl: &hdl
    }))?;
    hdl.spawn(mm.map_err(|e| {
        // FIXME: restartability

        error!("ModemManager failed: {}", e);
        panic!("modemmanager failed");
    }));
    info!("Initializing control bot");
    let cb = core.run(ControlBot::new(InitParameters {
        cfg: &config,
        store: store.clone(),
        cm: &mut cm,
        hdl: &hdl
    }))?;
    hdl.spawn(cb.map_err(|e| {
        // FIXME: restartability

        error!("ControlBot failed: {}", e);
        panic!("controlbot failed");
    }));
    info!("Initializing contact factory");
    let cf = ContactFactory::new(config, store, cm, hdl);
    let _ = core.run(cf.map_err(|e| {
        error!("ContactFactory failed: {}", e);
        panic!("contactfactory failed");
    }));
    Ok(())
}
