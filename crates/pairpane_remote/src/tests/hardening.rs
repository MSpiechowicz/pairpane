//! What a server cannot make the client do.

use pretty_assertions::assert_eq;

use super::Harness;
use crate::test_server::Settings;

const PASSWORD: &str = "correct horse";

#[test]
fn a_channel_the_server_opens_towards_the_client_is_refused() {
    let h = Harness::new(Settings::default());

    h.trust();
    h.site(Some(PASSWORD));

    // Forwarded ports (TCP and socket), an agent, X11, a session, and the two kinds of direct connection.
    assert_eq!(h.server.open_towards_clients(), [false; 7]);
}
