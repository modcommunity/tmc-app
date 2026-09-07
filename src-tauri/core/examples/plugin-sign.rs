//! Sign a plugin bundle, and check one.
//!
//! The other half of `tmc_core::plugins::signature`. The app verifies; a
//! publisher has to be able to produce, and without this the signature feature
//! is one somebody would have to reimplement from a doc comment to use — which
//! means guessing at the one detail that matters and getting it wrong.
//!
//! **What is signed is the manifest's CANONICAL bytes**, not the file as
//! written. `serde_json::to_vec` of the parsed manifest, which is exactly what
//! `Manifest::fingerprint` hashes. Two consequences, and they are the reason
//! this is a program rather than a line in a README:
//!
//!   * a bundle reformatted on the way to a user — a CDN that pretty-prints, an
//!     editor that reorders keys, a zip round trip — keeps its signature;
//!   * a bundle whose declared permissions or steps changed loses it, which is
//!     the entire point.
//!
//! ```text
//!   cargo run -p tmc-core --example plugin-sign -- keygen
//!   cargo run -p tmc-core --example plugin-sign -- sign   ./my-plugin <secret hex>
//!   cargo run -p tmc-core --example plugin-sign -- verify ./my-plugin <public hex>
//! ```
//!
//! The secret key is printed once and never stored by this tool. Where a
//! publisher keeps it is their problem and deliberately not this program's:
//! anything that wrote it to a default path would be a default path that gets
//! committed.

use std::path::Path;
use std::process::ExitCode;

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};

use tmc_core::plugins::manifest::{Manifest, MANIFEST_FILE};
use tmc_core::plugins::signature::SIGNATURE_FILE;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let result = match args.first().map(String::as_str) {
        Some("keygen") => keygen(),
        Some("sign") => sign(args.get(1), args.get(2)),
        Some("verify") => verify(args.get(1), args.get(2)),
        _ => {
            usage();

            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");

            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage:\n  \
         plugin-sign keygen\n  \
         plugin-sign sign   <bundle dir> <secret key hex>\n  \
         plugin-sign verify <bundle dir> <public key hex>"
    );
}

/// A fresh key pair, from the OS.
///
/// `getrandom` rather than a seed the caller supplies: a signing key derived
/// from anything a person can type is a signing key somebody else can guess,
/// and the one job this has is not to make that easy.
fn keygen() -> Result<(), String> {
    let mut seed = [0u8; 32];

    getrandom::getrandom(&mut seed).map_err(|e| format!("no system randomness: {e}"))?;

    let signing = SigningKey::from_bytes(&seed);

    println!("secret {}", hex::encode(signing.to_bytes()));
    println!("public {}", hex::encode(signing.verifying_key().to_bytes()));
    println!();
    println!("Keep the secret. Give the public key to whoever should trust you —");
    println!("they add it under Settings → Plugins → Trusted publishers.");

    Ok(())
}

fn sign(dir: Option<&String>, secret: Option<&String>) -> Result<(), String> {
    let (dir, secret) = (
        dir.ok_or("sign needs a bundle directory")?,
        secret.ok_or("sign needs a secret key")?,
    );

    let dir = Path::new(dir);
    let canonical = canonical_manifest(dir)?;

    let raw = hex::decode(secret.trim()).map_err(|_| "the secret key is not hexadecimal")?;

    let bytes: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "an Ed25519 secret key is 32 bytes — 64 hex characters")?;

    let signature = SigningKey::from_bytes(&bytes).sign(&canonical);

    let path = dir.join(SIGNATURE_FILE);

    std::fs::write(&path, hex::encode(signature.to_bytes()))
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;

    println!("wrote {}", path.display());

    Ok(())
}

fn verify(dir: Option<&String>, public: Option<&String>) -> Result<(), String> {
    let (dir, public) = (
        dir.ok_or("verify needs a bundle directory")?,
        public.ok_or("verify needs a public key")?,
    );

    let dir = Path::new(dir);
    let canonical = canonical_manifest(dir)?;

    let raw = hex::decode(public.trim()).map_err(|_| "the public key is not hexadecimal")?;

    let bytes: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "an Ed25519 public key is 32 bytes — 64 hex characters")?;

    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| "that is not a valid public key")?;

    let signature = tmc_core::plugins::signature::read_signature(dir)
        .map_err(|e| e.to_string())?
        .ok_or("that bundle has no plugin.sig")?;

    let signature: [u8; 64] = signature
        .as_slice()
        .try_into()
        .map_err(|_| "the signature is the wrong length")?;

    key.verify(
        &canonical,
        &ed25519_dalek::Signature::from_bytes(&signature),
    )
    .map_err(|_| "the signature does NOT match this key and this manifest".to_string())?;

    println!("ok — signed by this key, over this manifest");

    Ok(())
}

/// The bytes both halves operate on.
///
/// Parsed and re-serialised, never read as text. This is the whole contract,
/// and having one function produce it for signing and verifying alike is what
/// stops the two drifting.
fn canonical_manifest(dir: &Path) -> Result<Vec<u8>, String> {
    let path = dir.join(MANIFEST_FILE);

    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;

    let manifest = Manifest::parse(&raw).map_err(|e| format!("{path:?} is not valid: {e}"))?;

    serde_json::to_vec(&manifest).map_err(|e| format!("could not canonicalise the manifest: {e}"))
}
