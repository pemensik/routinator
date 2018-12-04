//! Configuration.

use std::{env, fmt, fs, io, process};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use clap::{App, Arg, ArgMatches};
use dirs::home_dir;
use fern;
use log::LevelFilter;
use syslog::Facility;
use toml;
use crate::operation::Error;
use crate::repository::Repository;
use crate::slurm::LocalExceptions;

//------------ Defaults for Some Values --------------------------------------

const DEFAULT_STRICT: bool = false;
const DEFAULT_RSYNC_COUNT: usize = 4;
const DEFAULT_REFRESH: u64 = 3600;
const DEFAULT_RETRY: u64 = 600;
const DEFAULT_EXPIRE: u64 = 7200;
const DEFAULT_HISTORY_SIZE: usize = 10;


//------------ Config --------------------------------------------------------

/// Routinator configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Path to the directory that contains the repository cache.
    pub cache_dir: PathBuf,

    /// Path to the directory that contains the trust anchor locators.
    pub tal_dir: PathBuf,

    /// Paths to the local exceptions files.
    pub exceptions: Vec<PathBuf>,

    /// Should we do strict validation?
    pub strict: bool,

    /// Number of parallel rsync commands.
    pub rsync_count: usize,

    /// Number of parallel validations.
    pub validation_threads: usize,

    /// The refresh interval for repository validation.
    pub refresh: Duration,

    /// The RTR retry inverval to be announced to a client.
    pub retry: Duration,

    /// The RTR expire time to be announced to a client.
    pub expire: Duration,

    /// How many diffs to keep in the history.
    pub history_size: usize,

    /// Addresses to listen for RTR TCP transport connections on.
    pub tcp_listen: Vec<SocketAddr>,

    /// The log levels to be logged.
    pub log_level: LevelFilter,

    /// Should we log to stderr?
    pub log_target: LogTarget,
}

impl Config {
    /// Creates a clap app that can parse the configuration.
    pub fn config_args<'a: 'b, 'b>(app: App<'a, 'b>) -> App<'a, 'b> {
        app
        .arg(Arg::with_name("config")
             .short("c")
             .long("config")
             .takes_value(true)
             .value_name("PATH")
             .help("read base configuration from this file")
        )
        .arg(Arg::with_name("base-dir")
             .short("b")
             .long("base-dir")
             .value_name("DIR")
             .help("sets the base directory for cache and TALs")
             .takes_value(true)
        )
        .arg(Arg::with_name("repository-dir")
             .short("r")
             .long("repository-dir")
             .value_name("DIR")
             .help("sets the repository cache directory")
             .takes_value(true)
        )
        .arg(Arg::with_name("tal-dir")
             .short("t")
             .long("tal-dir")
             .value_name("DIR")
             .help("sets the TAL directory")
             .takes_value(true)
        )
        .arg(Arg::with_name("exceptions")
             .short("x")
             .long("exceptions")
             .value_name("FILE")
             .help("file with local exceptions (see RFC 8416 for format)")
             .takes_value(true)
             .multiple(true)
        )
        .arg(Arg::with_name("strict")
             .long("strict")
             .help("parse RPKI data in strict mode")
        )
        .arg(Arg::with_name("rsync-count")
             .long("rsync-count")
             .value_name("COUNT")
             .help("number of parallel rsync commands")
             .takes_value(true)
        )
        .arg(Arg::with_name("validation-threads")
             .long("validation-threads")
             .value_name("COUNT")
             .help("number of threads for validation")
             .takes_value(true)
        )
        .arg(Arg::with_name("refresh")
             .long("refresh")
             .value_name("SECONDS")
             .default_value("3600")
             .help("refresh interval in seconds")
        )
        .arg(Arg::with_name("retry")
             .long("retry")
             .value_name("SECONDS")
             .default_value("600")
             .help("RTR retry interval in seconds")
        )
        .arg(Arg::with_name("expire")
             .long("expire")
             .value_name("SECONDS")
             .default_value("600")
             .help("RTR expire interval in seconds")
        )
        .arg(Arg::with_name("history")
             .long("history")
             .value_name("COUNT")
             .default_value("10")
             .help("number of history items to keep in repeat mode")
        )
        .arg(Arg::with_name("listen")
             .short("l")
             .long("listen")
             .value_name("ADDR:PORT")
             .help("listen addr:port for RTR.")
             .takes_value(true)
             .multiple(true)
        )
        .arg(Arg::with_name("verbose")
             .short("v")
             .long("verbose")
             .multiple(true)
             .help("log more information, twice for even more")
        )
        .arg(Arg::with_name("quiet")
             .short("q")
             .long("quiet")
             .multiple(true)
             .conflicts_with("verbose")
             .help("log less informatio, twice for no information")
        )
        .arg(Arg::with_name("syslog")
             .long("syslog")
             .help("log to syslog")
        )
        .arg(Arg::with_name("syslog-facility")
             .long("syslog-facility")
             .takes_value(true)
             .default_value("daemon")
             .help("facility to use for syslog logging")
        )
        .arg(Arg::with_name("logfile")
             .long("logfile")
             .takes_value(true)
             .value_name("PATH")
             .help("log to this file")
        )
    }

    pub fn from_arg_matches(matches: &ArgMatches) -> Self {
        let cur_dir = match env::current_dir() {
            Ok(dir) => dir,
            Err(err) => {
                println!(
                    "Fatal: cannot get current directory ({}). Aborting.",
                    err
                );
                process::exit(1);
            }
        };

        let mut res = Self::create_base_config(
            Self::path_value_of(matches, "config", &cur_dir)
                .as_ref().map(AsRef::as_ref)
        );

        // cache_dir
        if let Some(dir) = matches.value_of("repository-dir") {
            res.cache_dir = cur_dir.join(dir)
        }
        else if let Some(dir) = matches.value_of("base-dir") {
            res.cache_dir = cur_dir.join(dir).join("repository")
        }

        // tal_dir
        if let Some(dir) = matches.value_of("tal-dir") {
            res.cache_dir = cur_dir.join(dir)
        }
        else if let Some(dir) = matches.value_of("base-dir") {
            res.cache_dir = cur_dir.join(dir).join("tals")
        }

        // expceptions
        if let Some(list) = matches.values_of("exceptions") {
            res.exceptions = list.map(|path| cur_dir.join(path)).collect()
        }

        // strict
        if matches.is_present("strict") {
            res.strict = true
        }

        // rsync_count
        if let Some(value) = from_str_value_of(matches, "rsync-count") {
            res.rsync_count = value
        }

        // validation_threads
        if let Some(value) = from_str_value_of(matches, "validation-threads") {
            res.validation_threads = value
        }

        // refresh
        if let Some(value) = from_str_value_of(matches, "refresh") {
            res.refresh = Duration::from_secs(value)
        }

        // retry
        if let Some(value) = from_str_value_of(matches, "retry") {
            res.retry = Duration::from_secs(value)
        }

        // expire
        if let Some(value) = from_str_value_of(matches, "expire") {
            res.expire = Duration::from_secs(value)
        }

        // history_size
        if let Some(value) = from_str_value_of(matches, "history") {
            res.history_size = value
        }

        // tcp_listen
        if let Some(list) = matches.values_of("listen") {
            res.tcp_listen = list.map(|value| {
                match SocketAddr::from_str(value) {
                    Ok(some) => some,
                    Err(_) => {
                        println!("Invalid value for listen: {}", value);
                        process::exit(1);
                    }
                }
            }).collect()
        }

        // log_level
        match (matches.occurrences_of("verbose"),
                                            matches.occurrences_of("quiet")) {
            // This assumes that -v and -q are conflicting.
            (0, 0) => { }
            (1, 0) => res.log_level = LevelFilter::Info,
            (_, 0) => res.log_level = LevelFilter::Debug,
            (0, 1) => res.log_level = LevelFilter::Error,
            (0, _) => res.log_level = LevelFilter::Off,
            _ => { }
        }

        // log_target
        if matches.is_present("syslog") {
            res.log_target = LogTarget::Syslog(
                match Facility::from_str(
                               matches.value_of("syslog-facility").unwrap()) {
                    Ok(value) => value,
                    Err(_) => {
                        println!("Invalid value for syslog-facility.");
                        process::exit(1)
                    }
                }
            )
        }
        else if let Some(file) = matches.value_of("logfile") {
            if file == "-" {
                res.log_target = LogTarget::Stderr
            }
            else {
                res.log_target = LogTarget::File(cur_dir.join(file))
            }
        }

        res
    }

    /// Creates and returns the repository for this configuration.
    ///
    /// If `update` is `false`, all updates in the respository are disabled.
    ///
    /// If any errors happen, the method will print (!) an error message.
    pub fn create_repository(
        &self,
        update: bool
    ) -> Result<Repository, Error> {
        self.prepare_dirs();
        Repository::new(self, update).map_err(|err| {
            println!("{}", err);
            Error
        })
    }

    /// Loads the local exceptions for this configuration.
    ///
    /// If any errors happen, the method will print (!) an error message.
    pub fn load_exceptions(&self) -> Result<LocalExceptions, Error> {
        let mut res = LocalExceptions::empty();
        let mut ok = true;
        for path in &self.exceptions {
            if let Err(err) = res.extend_from_file(path) {
                println!(
                    "Failed to load exceptions file {}: {}",
                    path.display(), err
                );
                ok = false;
            }
        }
        if ok {
            Ok(res)
        }
        else {
            Err(Error)
        }
    }

    /// Switches logging to the configured target.
    ///
    /// If `daemon` is `true`, the default target is syslog, otherwise it is
    /// stderr.
    pub fn switch_logging(&self, daemon: bool) -> Result<(), Error> {
        match self.log_target {
            LogTarget::Default(fac) if daemon => {
                if let Err(err) = syslog::init(fac, self.log_level, None) {
                    println!("Failed to init syslog: {}", err);
                    return Err(Error)
                }
            }
            LogTarget::Syslog(fac) => {
                if let Err(err) = syslog::init(fac, self.log_level, None) {
                    println!("Failed to init syslog: {}", err);
                    return Err(Error)
                }
            }
            LogTarget::Stderr | LogTarget::Default(_) => {
                let dispatch = fern::Dispatch::new()
                    .level(self.log_level)
                    .chain(io::stderr());
                if let Err(err) = dispatch.apply() {
                    println!("Failed to init stderr logging: {}", err);
                    return Err(Error)
                }
            }
            LogTarget::File(ref path) => {
                let file = match fern::log_file(path) {
                    Ok(file) => file,
                    Err(err) => {
                        println!(
                            "Failed to open log file '{}': {}",
                            path.display(), err
                        );
                        return Err(Error)
                    }
                };
                let dispatch = fern::Dispatch::new()
                    .level(self.log_level)
                    .chain(file);
                if let Err(err) = dispatch.apply() {
                    println!("Failed to init file logging: {}", err);
                    return Err(Error)
                }
            }
        }
        Ok(())
    }

    /// Returns a path value in arg matches.
    ///
    /// This expands a relative path based on the given directory.
    fn path_value_of(
        matches: &ArgMatches,
        key: &str,
        dir: &Path
    ) -> Option<PathBuf> {
        matches.value_of(key).map(|path| dir.join(path))
    }

    /// Creates the correct base configuration for the given config file.
    /// 
    /// If no config path is given, tries to read the default config in
    /// `$HOME/.routinator.toml`. If that doesn’t exist, creates a default
    /// config.
    fn create_base_config(path: Option<&Path>) -> Self {
        let mut file = match path {
            Some(path) => {
                match ConfigFile::read(&path) {
                    Some(file) => file,
                    None => {
                        println!("Cannot read config file {}", path.display());
                        process::exit(1)
                    }
                }
            }
            None => {
                match home_dir() {
                    Some(dir) => match ConfigFile::read(
                                             &dir.join(".routinator.toml")) {
                        Some(file) => file,
                        None => return Self::default(),
                    }
                    None => return Self::default()
                }
            }
        };

        let facility = file.take_string("syslog-facility");
        let facility = facility.as_ref().map(AsRef::as_ref).unwrap_or("daemon");
        let facility = match Facility::from_str(facility) {
            Ok(value) => value,
            Err(_) => {
                println!(
                    "Error in config file {}: \
                     invalid syslog-facility.",
                     path.unwrap().display()
                );
                process::exit(1)
            }
        };
        let log_target = file.take_string("log");
        let log_target = match log_target.as_ref().map(AsRef::as_ref) {
            Some("default") | None => LogTarget::Default(facility),
            Some("syslog") => LogTarget::Syslog(facility),
            Some("stderr") =>  LogTarget::Stderr,
            Some("file") => {
                LogTarget::File(match file.take_path("log-file") {
                    Some(file) => file,
                    None => {
                        println!(
                            "Error in config file {}: \
                             log target \"file\" requires 'log-file' value.",
                             path.unwrap().display()
                        );
                        process::exit(1);
                    }
                })
            }
            Some(value) => {
                println!(
                    "Error in config file {}: \
                     invalid log target '{}'",
                     path.unwrap().display(),
                     value
                );
                process::exit(1);
            }
        };

        let res = Config {
            cache_dir: file.take_mandatory_path("repository-dir"),
            tal_dir: file.take_mandatory_path("tal-dir"),
            exceptions: file.take_path_array("exceptions"),
            strict: file.take_bool("strict").unwrap_or(false),
            rsync_count: {
                file.take_usize("rsync-count").unwrap_or(DEFAULT_RSYNC_COUNT)
            },
            validation_threads: {
                file.take_usize("validation-threads")
                    .unwrap_or(::num_cpus::get())
            },
            refresh: {
                Duration::from_secs(
                    file.take_u64("refresh").unwrap_or(DEFAULT_REFRESH)
                )
            },
            retry: {
                Duration::from_secs(
                    file.take_u64("retry").unwrap_or(DEFAULT_REFRESH)
                )
            },
            expire: {
                Duration::from_secs(
                    file.take_u64("expire").unwrap_or(DEFAULT_REFRESH)
                )
            },
            history_size: {
                file.take_usize("history-size").unwrap_or(DEFAULT_HISTORY_SIZE)
            },
            tcp_listen: file.take_from_str_array("listen-tcp"),
            log_level: {
                file.take_from_str("log-level").unwrap_or(LevelFilter::Warn)
            },
            log_target
        };
        file.check_exhausted();
        res
    }

    /// Creates a default config with the given paths.
    fn default_with_paths(cache_dir: PathBuf, tal_dir: PathBuf) -> Self {
        Config {
            cache_dir,
            tal_dir,
            exceptions: Vec::new(),
            strict: DEFAULT_STRICT,
            rsync_count: DEFAULT_RSYNC_COUNT,
            validation_threads: ::num_cpus::get(),
            refresh: Duration::from_secs(DEFAULT_REFRESH),
            retry: Duration::from_secs(DEFAULT_RETRY),
            expire: Duration::from_secs(DEFAULT_EXPIRE),
            history_size: DEFAULT_HISTORY_SIZE,
            tcp_listen: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 3323)
            ],
            log_level: LevelFilter::Warn,
            log_target: LogTarget::Stderr,
        }
    }


    /// Prepares and returns the cache dir and tal dir.
    fn prepare_dirs(&self) {
        if let Err(err) = fs::create_dir_all(&self.cache_dir) {
            println!(
                "Can't create repository directory {}: {}.\nAborting.",
                self.cache_dir.display(), err
            );
            process::exit(1);
        }
        if fs::read_dir(&self.tal_dir).is_err() {
            if let Err(err) = fs::create_dir_all(&self.tal_dir) {
                println!(
                    "Can't create TAL directory {}: {}.\nAborting.",
                    self.tal_dir.display(), err
                );
                process::exit(1);
            }
            for (name, content) in &DEFAULT_TALS {
                let mut file = match fs::File::create(self.tal_dir.join(name)) {
                    Ok(file) => file,
                    Err(err) => {
                        println!(
                            "Can't create TAL file {}: {}.\n Aborting.",
                            self.tal_dir.join(name).display(), err
                        );
                        process::exit(1);
                    }
                };
                if let Err(err) = file.write_all(content) {
                    println!(
                        "Can't create TAL file {}: {}.\n Aborting.",
                        self.tal_dir.join(name).display(), err
                    );
                    process::exit(1);
                }
            }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let base_dir = match home_dir() {
            Some(dir) => dir.join(".rpki-cache"),
            None => {
                println!(
                    "Cannot determine default directories \
                    (no home directory). Please specify \
                    explicitely."
                );
                process::exit(1);
            }
        };
        Config::default_with_paths(
            base_dir.join("repository"), 
            base_dir.join("tals")
        )
    }
}


//------------ LogTarget -----------------------------------------------------

/// The target to log to.
#[derive(Clone, Debug)]
pub enum LogTarget {
    /// Default.
    ///
    /// Logs to `Syslog(facility)? in daemon mode and `Stderr` otherwise.
    Default(Facility),

    /// Syslog.
    ///
    /// The argument is the syslog facility to use.
    Syslog(Facility),

    /// Stderr.
    Stderr,

    /// A file.
    ///
    /// The argument is the file name.
    File(PathBuf)
}


//------------ ConfigFile ----------------------------------------------------

/// The content of a config file.
///
/// This is a thin wrapper around `toml::Table` to make dealing with it more
/// convenient.
#[derive(Clone, Debug)]
struct ConfigFile {
    /// The content of the file.
    content: toml::value::Table,

    /// The path to the config file.
    path: PathBuf,

    /// The directory we found the file in.
    ///
    /// This is used in relative paths.
    dir: PathBuf,
}

impl ConfigFile {
    /// Reads the config file at the given path.
    ///
    /// If there is no such file, returns `None`. If there is a file but it
    /// is broken, aborts.
    fn read(path: &Path) -> Option<Self> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return None
        };
        let mut config = String::new();
        if let Err(err) = file.read_to_string(&mut config) {
            println!(
                "Failed to read config file {}: {}",
                path.display(), err
            );
            process::exit(1);
        }
        let content = match toml::from_str(&config) {
            Ok(toml::Value::Table(content)) => content,
            Ok(_) => {
                println!(
                    "Failed to parse config file {}: Not a mapping.",
                    path.display()
                );
                process::exit(1);
            }
            Err(err) => {
                println!(
                    "Failed to parse config file {}: {}",
                    path.display(), err
                );
                process::exit(1);
            }
        };
        let dir = if path.is_relative() {
            path.join(match env::current_dir() {
                Ok(dir) => dir,
                Err(err) => {
                    println!(
                        "Fatal: Can't determine current directory: {}.",
                        err
                    );
                    process::exit(1);
                }
            }).parent().unwrap().into() // a file always has a parent
        }
        else {
            path.parent().unwrap().into()
        };
        Some(ConfigFile {
            content,
            path: path.into(),
            dir: dir
        })
    }

    fn take_bool(&mut self, key: &str) -> Option<bool> {
        self.content.remove(key).map(|value| {
            if let toml::Value::Boolean(res) = value {
                res
            }
            else {
                println!(
                    "Error in config file {}: '{}' expected to be a boolean.",
                    self.path.display(), key
                );
                process::exit(1);
            }
        })
    }
    
    fn take_u64(&mut self, key: &str) -> Option<u64> {
        self.content.remove(key).map(|value| {
            if let toml::Value::Integer(res) = value {
                if res < 0 {
                    println!(
                        "Error in config file {}: \
                        '{}' expected to be a positive integer.",
                        self.path.display(), key
                    );
                    process::exit(1);
                }
                else {
                    res as u64
                }
            }
            else {
                println!(
                    "Error in config file {}: '{}' expected to be an integer.",
                    self.path.display(), key
                );
                process::exit(1);
            }
        })
    }

    fn take_usize(&mut self, key: &str) -> Option<usize> {
        self.content.remove(key).map(|value| {
            if let toml::Value::Integer(res) = value {
                if res < 0 {
                    println!(
                        "Error in config file {}: \
                        '{}' expected to be a positive integer.",
                        self.path.display(), key
                    );
                    process::exit(1);
                }
                if is_large_i64(res) {
                    println!(
                        "Error in config file {}: \
                        value for '{}' is too large.",
                        self.path.display(), key
                    );
                    process::exit(1);
                }
                res as usize
            }
            else {
                println!(
                    "Error in config file {}: '{}' expected to be a integer.",
                    self.path.display(), key
                );
                process::exit(1);
            }
        })
    }

    fn take_string(&mut self, key: &str) -> Option<String> {
        self.content.remove(key).map(|value| {
            if let toml::Value::String(res) = value {
                res
            }
            else {
                println!(
                    "Error in config file {}: '{}' expected to be a string.",
                    self.path.display(), key
                );
                process::exit(1);
            }
        })
    }

    fn take_from_str<T>(&mut self, key: &str) -> Option<T>
    where T: FromStr, T::Err: fmt::Display {
        self.take_string(key).map(|value| {
            match T::from_str(&value) {
                Ok(some) => some,
                Err(err) => {
                    println!(
                        "Error in config file {}: \
                         illegal value in '{}': {}.",
                        self.path.display(), key, err
                    );
                    process::exit(1)
                }
            }
        })
    }

    fn take_path(&mut self, key: &str) -> Option<PathBuf> {
        self.take_string(key).map(|path| self.dir.join(path))
    }

    fn take_mandatory_path(&mut self, key: &str) -> PathBuf {
        match self.take_path(key) {
            Some(res) => res,
            None => {
                println!(
                    "Error in config file {}: missing required '{}'.",
                    self.path.display(), key
                );
                process::exit(1)
            }
        }
    }

    fn take_path_array(&mut self, key: &str) -> Vec<PathBuf> {
        match self.content.remove(key) {
            Some(::toml::Value::Array(vec)) => {
                vec.into_iter().map(|value| {
                    if let ::toml::Value::String(value) = value {
                        self.dir.join(value)
                    }
                    else {
                        println!(
                            "Error in config file {}: \
                            '{}' expected to be a array of paths.",
                            self.path.display(),
                            key
                        );
                        process::exit(1);
                    }
                }).collect()
            }
            Some(_) => {
                println!(
                    "Error in config file {}: \
                     '{}' expected to be a array of paths.",
                    self.path.display(), key
                );
                process::exit(1);
            }
            None => return Vec::new()
        }
    }

    fn take_from_str_array<T>(&mut self, key: &str) -> Vec<T>
    where T: FromStr, T::Err: fmt::Display {
        match self.content.remove(key) {
            Some(::toml::Value::Array(vec)) => {
                vec.into_iter().map(|value| {
                    if let ::toml::Value::String(value) = value {
                        match T::from_str(&value) {
                            Ok(value) => value,
                            Err(err) => {
                                println!(
                                    "Error in config file {}: \
                                     Invalid value in '{}': {}",
                                    self.path.display(), key, err
                                );
                                process::exit(1)
                            }
                        }
                    }
                    else {
                        println!(
                            "Error in config file {}: \
                            '{}' expected to be a array of strings.",
                            self.path.display(),
                            key
                        );
                        process::exit(1);
                    }
                }).collect()
            }
            Some(_) => {
                println!(
                    "Error in config file {}: \
                     '{}' expected to be a array of strings.",
                    self.path.display(), key
                );
                process::exit(1);
            }
            None => return Vec::new()
        }
    }

    fn check_exhausted(&self) {
        if !self.content.is_empty() {
            print!(
                "Error in config file {}: Unknown settings ",
                self.path.display()
            );
            let mut first = true;
            for key in self.content.keys() {
                if !first {
                    print!(",");
                }
                else {
                    first = false
                }
                print!("{}", key);
            }
            println!(".");
            process::exit(1);
        }
    }
}


//------------ Helpers -------------------------------------------------------

fn from_str_value_of<T>(matches: &ArgMatches, key: &str) -> Option<T>
where T: FromStr, T::Err: fmt::Display {
    matches.value_of(key).map(|value| {
        match T::from_str(value) {
            Ok(value) => value,
            Err(err) => {
                println!(
                    "Invalid value for {}: {}.", 
                    key, err
                );
                process::exit(1);
            }
        }
    })
}

#[cfg(target_pointer_width = "32")]
fn is_large_i64(x: i64) -> bool {
    res > ::std::usize::MAX as i64
}

#[cfg(not(target_pointer_width = "32"))]
fn is_large_i64(_: i64) -> bool {
    false
}


//------------ DEFAULT_TALS --------------------------------------------------

const DEFAULT_TALS: [(&str, &[u8]); 5] = [
    ("afrinic.tal", include_bytes!("../tals/afrinic.tal")),
    ("apnic.tal", include_bytes!("../tals/apnic.tal")),
    ("arin.tal", include_bytes!("../tals/arin.tal")),
    ("lacnic.tal", include_bytes!("../tals/lacnic.tal")),
    ("ripe.tal", include_bytes!("../tals/ripe.tal")),
];

