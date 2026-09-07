//! TeamSpeak 3 ServerQuery — a line-oriented text protocol over TCP.
//!
//! Ported from `spy/internal/protocols/teamspeak3.go`, which was rewritten and
//! given 18 tests immediately before this. Where the two differ, the difference
//! is deliberate and called out below.
//!
//! ## The shape of a session
//!
//! ```text
//! server> TS3\n\r
//! server> Welcome to the TeamSpeak 3 ServerQuery interface...\n\r
//! client> use port=9987\n
//! server> error id=0 msg=ok\n\r
//! client> serverinfo\n
//! server> virtualserver_name=My\sServer virtualserver_maxclients=32 ...\n\r
//! server> error id=0 msg=ok\n\r
//! ```
//!
//! Every reply ends with a status line, and **the status line is the only frame
//! marker there is** — there is no length prefix and no sentinel byte. That is
//! what makes the read caps below load-bearing rather than defensive: a server
//! that answers and never sends a status line is asking us to read forever.
//!
//! ## Two ports, and they are not interchangeable
//!
//! ServerQuery listens on **10011**, once per machine. The thing a player joins
//! is a *virtual server* on its own voice port (9987 by default), and one
//! ServerQuery port fronts all of them. So a query needs both: connect to
//! 10011, then `use port=<voice port>` to pick which virtual server the
//! subsequent commands are about. Sending `serverinfo` without `use` first gets
//! `error id=1794 msg=server\sis\snot\srunning` — the machine is up, no virtual
//! server is selected, and the row would read as offline.
//!
//! This is why [`query`] takes `game_port` separately from `addr`. Every other
//! protocol in this module needs only the address it connected to.
//!
//! ## Counting players
//!
//! `spy` reports the roster length as the player count. That over-counts:
//! `clientlist` includes ServerQuery clients (`client_type=1`) — every
//! monitoring bot, every admin panel, and `spy`'s own scanner while it is
//! connected. A server watched by three tools reads as three players busier
//! than it is.
//!
//! Here the count comes from the server's own
//! `virtualserver_clientsonline - virtualserver_queryclientsonline`, which is
//! what the TeamSpeak client itself displays, and the roster is filtered to
//! `client_type=0`. The roster length is the fallback when `serverinfo` omits
//! the fields.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::transport::elapsed_ms;

/// The default ServerQuery port, used when a row carries no explicit one.
pub const DEFAULT_QUERY_PORT: u16 = 10_011;

/// Total bytes accepted for one command's reply.
///
/// `serverinfo` is a couple of KB; `clientlist` grows with the player count and
/// tops out well under this on a maxed-out server. Far above any honest reply,
/// far below what reading an unbounded stream off a hostile socket costs.
const MAX_RESPONSE: usize = 1 << 20;

/// Bytes accepted for one line before the reply is abandoned.
///
/// Without this, a server that opens with megabytes and no newline allocates on
/// our behalf for as long as it cares to.
const MAX_LINE: usize = 64 << 10;

/// Roster entries kept. A TeamSpeak virtual server can be licensed for far more
/// than any game server, but the cap stops a hostile `clientlist` from driving
/// an unbounded allocation of `PlayerEntry`.
const MAX_PLAYERS: usize = 2_048;

pub async fn query(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
    game_port: u16,
) -> AppResult<ServerQueryResult> {
    let started = Instant::now();

    let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| AppError::Network("The server did not answer.".into()))??;

    // Nagle would hold each short command line waiting for more that is never
    // coming, adding ~40ms to every round trip in a session that makes three.
    let _ = stream.set_nodelay(true);

    let rtt = elapsed_ms(started);

    let mut session = Session {
        stream,
        pending: Vec::with_capacity(4096),
    };

    session.read_banner(timeout).await?;

    /*
     * `use` selects the virtual server; see the module docs. Its own reply is a
     * bare status line, so the returned rows are empty and discarded — what
     * matters is that it did not error.
     */
    session
        .command(&format!("use port={game_port}"), timeout)
        .await
        .map_err(|_| AppError::invalid("No TeamSpeak server is running on that voice port."))?;

    let info = session.command("serverinfo", timeout).await?;
    let info = info.first().cloned().unwrap_or_default();

    let mut out = ServerQueryResult::offline(QueryProtocol::Teamspeak3, addr.port());
    out.online = true;
    out.rtt_ms = rtt;
    out.name = info
        .get("virtualserver_name")
        .cloned()
        .filter(|s| !s.is_empty());
    out.version = info
        .get("virtualserver_version")
        .cloned()
        .filter(|s| !s.is_empty());
    out.max_players = info
        .get("virtualserver_maxclients")
        .and_then(|v| v.parse().ok());
    out.password = info.get("virtualserver_flag_password").map(|v| v == "1");

    // The server's own count, less the monitoring tools connected to it. See
    // the module docs on why the roster length is the wrong number.
    out.players = players_online(&info);

    /*
     * The roster is a second round trip, so it is only made when the caller
     * asked — a grid of fifty cards does not want it. `clientlist` also needs a
     * ServerQuery login on most public servers while `serverinfo` does not, so
     * the error is swallowed: a server that answers one and refuses the other
     * must still render as online, with the count `serverinfo` already gave.
     */
    if want_players {
        if let Ok(rows) = session.command("clientlist", timeout).await {
            out.player_list = parse_roster(&rows);

            if out.players.is_none() {
                out.players = Some(out.player_list.len() as u32);
            }
        }
    }

    Ok(out)
}

/// One ServerQuery connection, plus whatever has been read past the last line.
///
/// The buffer lives on the session rather than on each command, because bytes
/// read past the end of the banner are the head of the first command's reply.
/// Discarding them along with a per-command reader is exactly the bug the Go
/// implementation carried.
struct Session {
    stream: TcpStream,
    pending: Vec<u8>,
}

impl Session {
    /// Consumes the two-line greeting and checks this is really ServerQuery.
    async fn read_banner(&mut self, timeout: Duration) -> AppResult<()> {
        let first = self.read_line(timeout).await?;

        if !first.starts_with("TS3") {
            return Err(AppError::invalid("That is not a TeamSpeak 3 query port."));
        }

        // The second line is human-readable blurb, and is only read to get it
        // out of the way of the first command's reply.
        let _ = self.read_line(timeout).await?;

        Ok(())
    }

    /// Sends one command and collects its reply up to the status line.
    async fn command(
        &mut self,
        cmd: &str,
        timeout: Duration,
    ) -> AppResult<Vec<BTreeMap<String, String>>> {
        let line = format!("{cmd}\n");

        tokio::time::timeout(timeout, self.stream.write_all(line.as_bytes()))
            .await
            .map_err(|_| AppError::Network("The server stopped responding.".into()))??;

        let mut body: Vec<String> = Vec::new();
        let mut total = 0usize;

        /*
         * The deadline covers the whole reply, not each read. Bounding only the
         * individual read bounds nothing here: a server dribbling one short
         * line every few milliseconds refreshes a per-read timeout forever
         * while never sending the status line that ends the reply, and the
         * byte cap below is then the only thing that stops it — a megabyte at
         * ten bytes a tick is a minute of holding the connection open.
         */
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());

            if remaining.is_zero() {
                return Err(AppError::Network("The server stopped responding.".into()));
            }

            let line = self.read_line(remaining).await?;

            total = total.saturating_add(line.len());
            if total > MAX_RESPONSE {
                return Err(AppError::invalid("The server sent an oversized reply."));
            }

            /*
             * The status line closes the reply, and is matched as a prefix of
             * its own line rather than as a substring of everything read so
             * far. The substring form rescans the whole accumulated reply once
             * per line, which is quadratic in the size of a `clientlist` — the
             * one reply here that scales with a server's player count.
             */
            if line.starts_with("error id=") {
                if !line.starts_with("error id=0 msg=ok") {
                    return Err(AppError::invalid("The server refused the query."));
                }
                break;
            }

            if !line.is_empty() {
                body.push(line);
            }
        }

        Ok(parse_response(&body.join("\n")))
    }

    /// Reads one `\n`-delimited line, refusing any that runs past [`MAX_LINE`].
    async fn read_line(&mut self, timeout: Duration) -> AppResult<String> {
        loop {
            if let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=pos).collect();

                /*
                 * TS3 terminates lines with "\n\r" rather than "\r\n", so the
                 * CR belongs to the front of the NEXT line. Trimming both ends
                 * of every line keeps that stray CR out of the field parser,
                 * where it would otherwise become part of the first key.
                 */
                return Ok(String::from_utf8_lossy(&line).trim().to_string());
            }

            if self.pending.len() > MAX_LINE {
                return Err(AppError::invalid("The server sent an oversized line."));
            }

            let mut chunk = [0u8; 4096];

            let read = tokio::time::timeout(timeout, self.stream.read(&mut chunk))
                .await
                .map_err(|_| AppError::Network("The server stopped responding.".into()))??;

            if read == 0 {
                return Err(AppError::Network(
                    "The server closed the connection.".into(),
                ));
            }

            self.pending.extend_from_slice(&chunk[..read]);
        }
    }
}

/// Player count from `serverinfo`, excluding the tools watching the server.
fn players_online(info: &BTreeMap<String, String>) -> Option<u32> {
    let online: u32 = info.get("virtualserver_clientsonline")?.parse().ok()?;

    let query: u32 = info
        .get("virtualserver_queryclientsonline")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    Some(online.saturating_sub(query))
}

/// Voice clients from a `clientlist` reply, in the order the server gave them.
fn parse_roster(rows: &[BTreeMap<String, String>]) -> Vec<PlayerEntry> {
    rows.iter()
        // client_type 1 is a ServerQuery connection, not somebody in a channel.
        // Absent means 0 on servers that omit it.
        .filter(|row| row.get("client_type").map(|t| t != "1").unwrap_or(true))
        .filter_map(|row| {
            let name = row.get("client_nickname")?.trim();

            if name.is_empty() {
                return None;
            }

            // A bare `clientlist` reports none of the other three. They stay
            // `None` rather than 0 so the UI shows a name and no columns,
            // instead of a roster of players who all scored nothing.
            Some(PlayerEntry {
                name: name.to_string(),
                score: None,
                duration: None,
                ping: None,
            })
        })
        .take(MAX_PLAYERS)
        .collect()
}

/// Splits a reply body into rows of `key=value` pairs.
///
/// Rows are pipe-separated and fields within a row are space-separated, which
/// works only because both characters are escaped inside values (`\p` and
/// `\s`). Unescaping therefore has to happen *after* the split, never before.
fn parse_response(body: &str) -> Vec<BTreeMap<String, String>> {
    body.split('|')
        .filter_map(|segment| {
            let segment = segment.trim();

            if segment.is_empty() {
                return None;
            }

            let row: BTreeMap<String, String> = segment
                .split(' ')
                .filter(|field| !field.trim().is_empty())
                .map(|field| match field.split_once('=') {
                    Some((key, value)) => (key.to_string(), unescape(value)),
                    None => (field.to_string(), String::new()),
                })
                .collect();

            (!row.is_empty()).then_some(row)
        })
        .collect()
}

/// Reverses ServerQuery's escaping, left to right.
///
/// It has to be a single scan. Chained independent replacements cannot express
/// `\\`, because an escaped backslash is what protects the character after it:
/// rewriting `\s` to a space first turns `50\\s` — a literal backslash followed
/// by an `s` — into `50\ `, inventing whitespace the server never sent. Going
/// left to right, the `\\` is consumed as one unit and the `s` after it is just
/// a letter.
fn unescape(value: &str) -> String {
    if !value.contains('\\') {
        return value.to_string();
    }

    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('s') => out.push(' '),
            Some('p') => out.push('|'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\x0b'),
            // Not an escape we know. Keep both characters rather than silently
            // eating the backslash and changing the name.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            // A trailing lone backslash is not an escape at all.
            None => out.push('\\'),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn unescapes_the_whole_table() {
        assert_eq!(unescape(r"My\sServer"), "My Server");
        assert_eq!(unescape(r"a\pb"), "a|b");
        assert_eq!(unescape(r"a\/b"), "a/b");
        assert_eq!(unescape(r"a\nb"), "a\nb");
        assert_eq!(unescape(r"a\rb"), "a\rb");
        assert_eq!(unescape(r"a\tb"), "a\tb");
        assert_eq!(unescape(r"a\vb"), "a\x0bb");
        assert_eq!(unescape(r"a\fb"), "a\x0cb");
        assert_eq!(unescape(r"a\bb"), "a\x08b");
        assert_eq!(unescape(r"a\ab"), "a\x07b");
    }

    #[test]
    fn an_escaped_backslash_protects_the_next_character() {
        // The case no chain of independent replacements can get right: the `s`
        // is a letter, not the tail of a `\s`.
        assert_eq!(unescape(r"50\\s"), r"50\s");
        assert_eq!(unescape(r"back\\slash"), r"back\slash");
    }

    #[test]
    fn leaves_unknown_escapes_alone() {
        assert_eq!(unescape(r"a\qb"), r"a\qb");
        assert_eq!(unescape(r"trailing\"), r"trailing\");
    }

    #[test]
    fn passes_through_values_with_no_escapes() {
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape(""), "");
    }

    #[test]
    fn parses_a_serverinfo_row() {
        let rows = parse_response(
            r"virtualserver_name=Test\sServer virtualserver_maxclients=32 virtualserver_clientsonline=5",
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["virtualserver_name"], "Test Server");
        assert_eq!(rows[0]["virtualserver_maxclients"], "32");
    }

    #[test]
    fn splits_pipe_separated_rows() {
        let rows = parse_response(r"clid=1 client_nickname=Ada|clid=2 client_nickname=Grace");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["client_nickname"], "Ada");
        assert_eq!(rows[1]["client_nickname"], "Grace");
    }

    #[test]
    fn a_pipe_inside_a_name_does_not_split_the_row() {
        // Escaped as \p on the wire, so the split sees one row and the
        // unescape puts the pipe back afterwards.
        let rows = parse_response(r"clid=1 client_nickname=Ada\pLovelace");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["client_nickname"], "Ada|Lovelace");
    }

    #[test]
    fn handles_a_valueless_field() {
        let rows = parse_response("clid=1 away");

        assert_eq!(rows[0]["away"], "");
    }

    #[test]
    fn ignores_empty_segments() {
        assert!(parse_response("").is_empty());
        assert!(parse_response("   ").is_empty());
        assert!(parse_response("||").is_empty());
    }

    #[test]
    fn counts_players_without_the_monitoring_bots() {
        let info = row(&[
            ("virtualserver_clientsonline", "7"),
            ("virtualserver_queryclientsonline", "3"),
        ]);

        assert_eq!(players_online(&info), Some(4));
    }

    #[test]
    fn counts_players_when_the_server_omits_the_query_count() {
        let info = row(&[("virtualserver_clientsonline", "7")]);

        assert_eq!(players_online(&info), Some(7));
    }

    #[test]
    fn a_query_count_above_the_total_floors_at_zero() {
        let info = row(&[
            ("virtualserver_clientsonline", "1"),
            ("virtualserver_queryclientsonline", "4"),
        ]);

        assert_eq!(players_online(&info), Some(0));
    }

    #[test]
    fn no_count_without_the_field() {
        assert_eq!(players_online(&row(&[])), None);
        assert_eq!(
            players_online(&row(&[("virtualserver_clientsonline", "lots")])),
            None
        );
    }

    #[test]
    fn the_roster_drops_query_clients() {
        let rows = vec![
            row(&[("client_nickname", "Ada"), ("client_type", "0")]),
            row(&[("client_nickname", "serveradmin"), ("client_type", "1")]),
            // No client_type at all: a voice client on servers that omit it.
            row(&[("client_nickname", "Grace")]),
        ];

        let roster = parse_roster(&rows);

        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].name, "Ada");
        assert_eq!(roster[1].name, "Grace");
    }

    #[test]
    fn the_roster_skips_rows_with_no_usable_name() {
        let rows = vec![
            row(&[("clid", "1")]),
            row(&[("client_nickname", "   ")]),
            row(&[("client_nickname", "Ada")]),
        ];

        assert_eq!(parse_roster(&rows).len(), 1);
    }

    #[test]
    fn the_roster_is_capped() {
        let rows: Vec<_> = (0..MAX_PLAYERS + 50)
            .map(|i| row(&[("client_nickname", "x")]).tap(i))
            .collect();

        assert_eq!(parse_roster(&rows).len(), MAX_PLAYERS);
    }

    /// Lets the cap test build distinct rows without a second helper.
    trait Tap {
        fn tap(self, i: usize) -> Self;
    }

    impl Tap for BTreeMap<String, String> {
        fn tap(mut self, i: usize) -> Self {
            self.insert("clid".into(), i.to_string());
            self
        }
    }

    // ---- Against a real socket ------------------------------------------
    //
    // The parser tests above cannot catch a framing bug, and framing is where
    // this protocol is actually hard: there is no length prefix, so the status
    // line is the only thing that ends a reply. These drive the whole session
    // over loopback instead.

    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::TcpListener;

    /// A ServerQuery server that replies from a script, one entry per command.
    ///
    /// `chunks` controls how the banner is written: sending it in pieces is
    /// what proves the session buffer survives a reply split across reads.
    async fn mock_server(script: Vec<(&'static str, &'static str)>, split_banner: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write) = stream.into_split();

            if split_banner {
                // The greeting arrives in two writes, and the second carries
                // the head of the first command's reply behind it.
                write.write_all(b"TS3\n\r").await.unwrap();
                tokio::time::sleep(Duration::from_millis(30)).await;
                write
                    .write_all(b"Welcome to ServerQuery\n\r")
                    .await
                    .unwrap();
            } else {
                write
                    .write_all(b"TS3\n\rWelcome to ServerQuery\n\r")
                    .await
                    .unwrap();
            }

            let mut lines = BufReader::new(read_half).lines();
            let mut step = 0usize;

            while let Ok(Some(line)) = lines.next_line().await {
                let Some((expect, reply)) = script.get(step) else {
                    break;
                };

                assert_eq!(line.trim(), *expect, "unexpected command at step {step}");
                write.write_all(reply.as_bytes()).await.unwrap();
                step += 1;
            }
        });

        port
    }

    fn addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{port}").parse().unwrap()
    }

    const OK: &str = "error id=0 msg=ok\n\r";

    #[tokio::test]
    async fn queries_a_server_end_to_end() {
        let port = mock_server(
            vec![
                ("use port=9987", OK),
                (
                    "serverinfo",
                    concat!(
                        "virtualserver_name=Ada\\sand\\sGrace ",
                        "virtualserver_maxclients=32 ",
                        "virtualserver_clientsonline=4 ",
                        "virtualserver_queryclientsonline=1 ",
                        "virtualserver_version=3.13.7 ",
                        "virtualserver_flag_password=1\n\r",
                        "error id=0 msg=ok\n\r",
                    ),
                ),
                (
                    "clientlist",
                    concat!(
                        "clid=1 client_nickname=Ada client_type=0|",
                        "clid=2 client_nickname=serveradmin client_type=1|",
                        "clid=3 client_nickname=Grace client_type=0\n\r",
                        "error id=0 msg=ok\n\r",
                    ),
                ),
            ],
            false,
        )
        .await;

        let out = query(addr(port), Duration::from_secs(5), true, 9987)
            .await
            .unwrap();

        assert!(out.online);
        assert_eq!(out.name.as_deref(), Some("Ada and Grace"));
        assert_eq!(out.max_players, Some(32));
        assert_eq!(out.version.as_deref(), Some("3.13.7"));
        assert_eq!(out.password, Some(true));

        // 4 online less the 1 query client, NOT the roster length.
        assert_eq!(out.players, Some(3));

        // ...and the roster itself has the query client filtered out, so it is
        // deliberately shorter than the count above.
        assert_eq!(out.player_list.len(), 2);
        assert_eq!(out.player_list[0].name, "Ada");
        assert_eq!(out.player_list[1].name, "Grace");
    }

    #[tokio::test]
    async fn survives_a_banner_split_across_reads() {
        let port = mock_server(
            vec![
                ("use port=9987", OK),
                (
                    "serverinfo",
                    "virtualserver_name=Split virtualserver_maxclients=8\n\rerror id=0 msg=ok\n\r",
                ),
            ],
            true,
        )
        .await;

        let out = query(addr(port), Duration::from_secs(5), false, 9987)
            .await
            .unwrap();

        assert_eq!(out.name.as_deref(), Some("Split"));
        assert_eq!(out.max_players, Some(8));
    }

    #[tokio::test]
    async fn skips_the_roster_when_the_caller_did_not_ask() {
        let port = mock_server(
            vec![
                ("use port=9987", OK),
                (
                    "serverinfo",
                    "virtualserver_name=Quiet virtualserver_clientsonline=2\n\rerror id=0 msg=ok\n\r",
                ),
                // No clientlist entry: the mock asserts on the command it gets,
                // so a stray third round trip fails this test rather than
                // passing silently.
            ],
            false,
        )
        .await;

        let out = query(addr(port), Duration::from_secs(5), false, 9987)
            .await
            .unwrap();

        assert_eq!(out.players, Some(2));
        assert!(out.player_list.is_empty());
    }

    #[tokio::test]
    async fn a_refused_roster_still_leaves_the_server_online() {
        // Public servers commonly allow serverinfo and refuse clientlist. That
        // must not turn into an offline row.
        let port = mock_server(
            vec![
                ("use port=9987", OK),
                (
                    "serverinfo",
                    "virtualserver_name=Locked virtualserver_clientsonline=9\n\rerror id=0 msg=ok\n\r",
                ),
                (
                    "clientlist",
                    "error id=2568 msg=insufficient\\sclient\\spermissions\n\r",
                ),
            ],
            false,
        )
        .await;

        let out = query(addr(port), Duration::from_secs(5), true, 9987)
            .await
            .unwrap();

        assert!(out.online);
        assert_eq!(out.name.as_deref(), Some("Locked"));
        assert_eq!(out.players, Some(9));
        assert!(out.player_list.is_empty());
    }

    #[tokio::test]
    async fn a_missing_virtual_server_is_an_error_not_an_empty_row() {
        let port = mock_server(
            vec![(
                "use port=9987",
                "error id=1794 msg=server\\sis\\snot\\srunning\n\r",
            )],
            false,
        )
        .await;

        let err = query(addr(port), Duration::from_secs(5), false, 9987).await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn refuses_a_port_that_is_not_serverquery() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // An HTTP server, say. Answers, but is not ServerQuery.
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        });

        let err = query(addr(port), Duration::from_secs(5), false, 9987).await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn a_server_that_never_sends_a_status_line_times_out() {
        // The framing hazard: no length prefix, so a reply with no status line
        // has no end. This must terminate on the deadline rather than read
        // forever.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream.write_all(b"TS3\n\rWelcome\n\r").await;

            loop {
                if stream.write_all(b"filler=1\n\r").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let started = Instant::now();
        let err = query(addr(port), Duration::from_millis(300), false, 9987).await;

        assert!(err.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the query did not honour its deadline"
        );
    }
}
