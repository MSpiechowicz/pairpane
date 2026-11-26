//! An SSH server with an SFTP service, in this process, on `127.0.0.1`: what the tests (and the app's tests, with the
//! `test-server` feature) connect to. It serves a temporary folder as `/`, with the sign-in methods the test chooses.
//! Its host key is made when it starts; nothing here is a real key, and nothing touches the user's files or agent.

mod files;
mod open;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::ssh_key::{Cipher, Kdf, LineEnding};
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{Auth, Msg, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;

use self::files::Files;

/// The user name the server accepts, and any other that begins with it (`me2`): a test that needs several sites on one
/// server connects as different users.
pub const USER: &str = "me";

/// The folder `realpath(".")` answers with: the user's home.
pub const HOME: &str = "/home/me";

/// A new Ed25519 key, made from the system's random bytes.
pub fn generate_key() -> PrivateKey {
    let mut seed = [0u8; 32];

    getrandom::fill(&mut seed).expect("random bytes");

    PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)), "test").expect("a key")
}

/// `key` encrypted with `passphrase` (the format of `ssh-keygen`), written to `path` as OpenSSH reads it.
pub fn write_key(key: &PrivateKey, passphrase: Option<&str>, path: &Path) {
    let key = match passphrase {
        None => key.clone(),
        Some(passphrase) => {
            let mut salt = vec![0u8; 16];

            getrandom::fill(&mut salt).expect("random bytes");

            // Few rounds: the tests ask for the passphrase more than once.
            key.encrypt_with(Cipher::Aes256Ctr, Kdf::Bcrypt { salt, rounds: 4 }, 7, passphrase)
                .expect("encrypted")
        }
    };

    std::fs::write(path, key.to_openssh(LineEnding::LF).expect("encoded").as_bytes()).expect("key written");
}

/// What the server is like.
#[derive(Clone)]
pub struct Settings {
    /// The password it accepts; `None` turns password sign-in off.
    pub password: Option<String>,
    /// Whether the same password is also taken as the answer to one hidden keyboard-interactive prompt.
    pub keyboard_interactive: bool,
    /// A two-step sign-in: the keyboard-interactive password is followed by a second hidden prompt (`Verification
    /// code:`), and the password method is off. The answers to the second prompt are in `counts.verification`.
    pub two_step: bool,
    /// The public key it accepts; `None` turns key sign-in off.
    pub authorized_key: Option<PublicKey>,
    /// How many entries a READDIR answer holds.
    pub page_size: usize,
    /// How long each READDIR answer is held back.
    pub page_delay: Duration,
    /// Paths (`/a/b`) whose OPENDIR, REMOVE and RMDIR are never answered.
    pub stalled: Vec<String>,
    /// The most bytes a READ answers with (the server sends less than was asked for); `None` for no limit.
    pub short_reads: Option<usize>,
    /// Whether READ is never answered.
    pub stall_reads: bool,
    /// A READ at or after this offset fails (the server could not read the file there); `None` for no failure.
    pub fail_reads_from: Option<u64>,
    /// After this many READ requests on a connection, READ waits (see [`Counts::release_reads`]); `None` for no limit.
    pub stall_reads_after: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            password: Some("correct horse".to_owned()),
            keyboard_interactive: false,
            two_step: false,
            authorized_key: None,
            page_size: 100,
            page_delay: Duration::ZERO,
            stalled: Vec::new(),
            short_reads: None,
            stall_reads: false,
            fail_reads_from: None,
            stall_reads_after: None,
        }
    }
}

/// What the server counts, for the tests to look at.
#[derive(Default)]
pub struct Counts {
    /// Connections accepted.
    pub connections: AtomicUsize,
    /// READDIR answers sent.
    pub pages: AtomicUsize,
    /// Files open for reading now, over all connections; a connection that ends lets go of its files.
    pub open_handles: AtomicUsize,
    /// READ requests received (before they are answered, or stalled).
    pub reads: AtomicUsize,
    /// Whether the READ requests that wait because of `stall_reads_after` are answered.
    pub reads_released: AtomicBool,
    /// REMOVE and RMDIR requests received (before they are answered, or stalled).
    pub removes: AtomicUsize,
    /// Connections that were hung up on at once, because the server was told to stop taking them.
    pub refused: AtomicUsize,
    /// Every password the server was offered (as a password or as the answer to a prompt), in order.
    pub passwords: Mutex<Vec<String>>,
    /// Every answer to the second prompt of a two-step sign-in, in order.
    pub verification: Mutex<Vec<String>>,
}

impl Counts {
    /// Lets the READ requests that wait because of `stall_reads_after` (and those to come) be answered.
    pub fn release_reads(&self) {
        self.reads_released.store(true, Ordering::Release);
    }

    /// The passwords offered so far.
    pub fn offered(&self) -> Vec<String> {
        self.passwords.lock().expect("passwords").clone()
    }
}

/// Hangs up on connections that are still being set up (before the client signed in), which
/// [`TestServer::drop_connections`] cannot reach.
#[derive(Clone)]
pub struct Killer {
    sockets: Arc<Mutex<Vec<std::net::TcpStream>>>,
}

impl Killer {
    /// Shuts every connection's socket at once, without a goodbye.
    pub fn kill(&self) {
        for socket in self.sockets.lock().expect("sockets").drain(..) {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// A running server. Dropping it stops it.
pub struct TestServer {
    pub addr: SocketAddr,
    /// The server's host key (the public half).
    pub host_key: PublicKey,
    /// The files it serves; a test fills it.
    pub root: TempDir,
    pub counts: Arc<Counts>,
    sessions: Arc<Mutex<Vec<russh::server::Handle>>>,
    sockets: Arc<Mutex<Vec<std::net::TcpStream>>>,
    accepting: Arc<AtomicBool>,
    runtime: Option<Runtime>,
}

impl TestServer {
    pub fn start(settings: Settings) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime");
        let root = tempfile::tempdir().expect("temp dir");
        let host = generate_key();
        let host_key = host.public_key().clone();
        let counts = Arc::new(Counts::default());
        let sessions = Arc::new(Mutex::new(Vec::new()));
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let accepting = Arc::new(AtomicBool::new(true));

        std::fs::create_dir_all(root.path().join(HOME.trim_start_matches('/'))).expect("home folder");

        let mut methods = Vec::new();

        if settings.password.is_some() {
            methods.push(MethodKind::Password);
        }

        if settings.keyboard_interactive {
            methods.push(MethodKind::KeyboardInteractive);
        }

        if settings.authorized_key.is_some() {
            methods.push(MethodKind::PublicKey);
        }

        let config = Arc::new(russh::server::Config {
            keys: vec![host],
            methods: MethodSet::from(methods.as_slice()),
            auth_rejection_time: Duration::from_millis(5),
            auth_rejection_time_initial: Some(Duration::ZERO),
            inactivity_timeout: None,
            ..russh::server::Config::default()
        });
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
        let addr = listener.local_addr().expect("an address");

        listener.set_nonblocking(true).expect("non-blocking");

        let mut server = Accept {
            settings: Arc::new(settings),
            root: root.path().to_owned(),
            counts: Arc::clone(&counts),
        };
        let handles = Arc::clone(&sessions);
        let kept = Arc::clone(&sockets);
        let open = Arc::clone(&accepting);
        let refused = Arc::clone(&counts);

        runtime.spawn(async move {
            let listener = TcpListener::from_std(listener).expect("a listener");

            while let Ok((stream, _)) = listener.accept().await {
                if !open.load(Ordering::Acquire) {
                    refused.refused.fetch_add(1, Ordering::Relaxed);

                    continue;
                }

                let handler = server.new_client(None);
                let config = Arc::clone(&config);
                let handles = Arc::clone(&handles);

                // A second handle on the socket, for `Killer`.
                let stream = match stream.into_std() {
                    Ok(std) => {
                        if let Ok(copy) = std.try_clone() {
                            kept.lock().expect("sockets").push(copy);
                        }

                        TcpStream::from_std(std).expect("a stream")
                    }
                    Err(_) => continue,
                };

                tokio::spawn(async move {
                    if let Ok(running) = russh::server::run_stream(config, stream, handler).await {
                        handles.lock().expect("sessions").push(running.handle());

                        let _ = running.await;
                    }
                });
            }
        });

        Self {
            addr,
            host_key,
            root,
            counts,
            sessions,
            sockets,
            accepting,
            runtime: Some(runtime),
        }
    }

    /// Hangs up on every client; the server goes on accepting new connections.
    pub fn drop_connections(&self) {
        let sessions: Vec<_> = self.sessions.lock().expect("sessions").drain(..).collect();

        self.runtime.as_ref().expect("a running server").block_on(async {
            for session in sessions {
                let _ = session
                    .disconnect(russh::Disconnect::ByApplication, String::new(), String::new())
                    .await;
            }
        });
    }

    /// Stops taking connections: a client that connects is hung up on at once (`counts.refused` counts them), as when
    /// the server is down.
    pub fn refuse_new_connections(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    /// Takes connections again after [`refuse_new_connections`](Self::refuse_new_connections).
    pub fn accept_new_connections(&self) {
        self.accepting.store(true, Ordering::Release);
    }

    /// A handle that hangs up on connections that are still being set up.
    pub fn killer(&self) -> Killer {
        Killer {
            sockets: Arc::clone(&self.sockets),
        }
    }

    /// Asks every signed-in client to open each kind of channel a server can open towards a client (forwarded ports, an
    /// agent, X11, a session, a direct connection); whether each was accepted.
    pub fn open_towards_clients(&self) -> Vec<bool> {
        let sessions: Vec<_> = self.sessions.lock().expect("sessions").clone();

        self.runtime.as_ref().expect("a running server").block_on(async {
            let mut accepted = Vec::new();

            for session in sessions {
                accepted.extend([
                    session
                        .channel_open_forwarded_tcpip("127.0.0.1", 1, "127.0.0.1", 2)
                        .await
                        .is_ok(),
                    session.channel_open_forwarded_streamlocal("/tmp/x").await.is_ok(),
                    session.channel_open_agent().await.is_ok(),
                    session.channel_open_x11("127.0.0.1", 3).await.is_ok(),
                    session.channel_open_session().await.is_ok(),
                    session
                        .channel_open_direct_tcpip("127.0.0.1", 4, "127.0.0.1", 5)
                        .await
                        .is_ok(),
                    session.channel_open_direct_streamlocal("/tmp/y").await.is_ok(),
                ]);
            }

            accepted
        })
    }

    /// Makes `name` (below `/`) with `content`, and the folders above it.
    pub fn put(&self, name: &str, content: &[u8]) {
        let path = self.root.path().join(name.trim_start_matches('/'));

        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        std::fs::write(path, content).expect("file");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Makes a handler for each connection.
#[derive(Clone)]
struct Accept {
    settings: Arc<Settings>,
    root: std::path::PathBuf,
    counts: Arc<Counts>,
}

impl russh::server::Server for Accept {
    type Handler = Connection;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> Connection {
        self.counts.connections.fetch_add(1, Ordering::Relaxed);

        Connection {
            settings: Arc::clone(&self.settings),
            files: Files::new(self.root.clone(), &self.settings, Arc::clone(&self.counts)),
            counts: Arc::clone(&self.counts),
            channels: Vec::new(),
            round: 0,
        }
    }
}

/// One client's connection.
struct Connection {
    settings: Arc<Settings>,
    files: Files,
    counts: Arc<Counts>,
    channels: Vec<Channel<Msg>>,
    /// How many questions of a keyboard-interactive exchange were answered.
    round: usize,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.files.end();
    }
}

fn reject() -> Auth {
    Auth::reject()
}

impl russh::server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.counts
            .passwords
            .lock()
            .expect("passwords")
            .push(password.to_owned());

        let accepted =
            !self.settings.two_step && user.starts_with(USER) && self.settings.password.as_deref() == Some(password);

        Ok(if accepted { Auth::Accept } else { reject() })
    }

    async fn auth_publickey(&mut self, user: &str, key: VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        let accepted = user.starts_with(USER)
            && self.settings.authorized_key.as_ref().map(PublicKey::key_data) == Some(key.public_key.key_data());

        Ok(if accepted { Auth::Accept } else { reject() })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<russh::server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(mut answers) = response else {
            self.round = 0;

            return Ok(Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: vec![("Password: ".into(), false)].into(),
            });
        };
        let given = answers.next();

        if self.settings.two_step && self.round > 0 {
            self.counts.verification.lock().expect("verification").push(
                given
                    .as_deref()
                    .map(|given| String::from_utf8_lossy(given).into_owned())
                    .unwrap_or_default(),
            );

            return Ok(reject());
        }

        self.round += 1;

        if let Some(given) = &given {
            self.counts
                .passwords
                .lock()
                .expect("passwords")
                .push(String::from_utf8_lossy(given).into_owned());
        }

        let password_given =
            given.as_deref().map(<[u8]>::to_vec) == self.settings.password.as_ref().map(|p| p.as_bytes().to_vec());

        if self.settings.two_step && password_given {
            return Ok(Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: vec![("Verification code: ".into(), false)].into(),
            });
        }

        let accepted = self.settings.keyboard_interactive && user.starts_with(USER) && password_given;

        Ok(if accepted { Auth::Accept } else { reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.push(channel);
        reply.accept().await;

        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        let position = self.channels.iter().position(|channel| channel.id() == id);

        match (name, position) {
            ("sftp", Some(position)) => {
                let channel = self.channels.remove(position);

                session.channel_success(id)?;
                russh_sftp::server::run(channel.into_stream(), self.files.clone()).await;
            }
            _ => session.channel_failure(id)?,
        }

        Ok(())
    }
}
