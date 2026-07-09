//! v1.0: end-to-end encryption — Double Ratchet(前向保密)。
//!
//! ## Cryptographic design
//!
//! * **Key exchange**: X25519 ECDH (Curve25519).  Each device holds
//!   a long-term X25519 key pair.  The public key is shared during
//!   the QR-code pairing flow; the private key never leaves the
//!   device.
//! * **Double Ratchet** (v2 信封): 结合 DH 棘轮 + KDF 链棘轮,
//!   实现前向保密(Forward Secrecy)。
//!   - **DH 棘轮**: 每当收到对端新的 DH 公钥时,生成新 DH 密钥对,
//!     新 ECDH 输出经 KDF_RK 更新根密钥。
//!   - **KDF 链棘轮**: 两次 DH 棘轮之间,链密钥经 KDF_CK 派生
//!     消息密钥,每条消息使用唯一密钥后丢弃,实现前向保密。
//! * **Key derivation**: HKDF-SHA256 over 32-byte secrets.
//! * **AEAD**: AES-256-GCM.  Each message is encrypted with a fresh
//!   12-byte random nonce; the 16-byte authentication tag is
//!   appended to the ciphertext.
//!
//! ## Wire format
//!
//! v1 信封(单棘轮,向后兼容):
//! ```json
//! { "v": 1, "salt": "b64(32)", "nonce": "b64(12)", "ct": "b64(ct+tag)" }
//! ```
//!
//! v2 信封(双棘轮):
//! ```json
//! { "v": 2, "dh": "b64(32)", "n": 0, "nonce": "b64(12)", "ct": "b64(ct+tag)" }
//! ```
//!
//! ## Threat model
//!
//! * **In scope**: passive eavesdropper on the transport (cannot
//!   decrypt), tampering (caught by the GCM tag), replay (the
//!   receiver tracks a "last seen seq" and rejects duplicates),
//!   **forward secrecy** (v2: 密钥泄露后旧消息无法解密)。
//! * **Out of scope**: active MITM during pairing (we assume the QR
//!   code is shown locally on both devices, so the user can
//!   visually confirm the fingerprint).
//!
//! ## Backward compatibility
//!
//! v1 信封仍可解密(SessionKey 路径)。跨设备新消息以 v2(双棘轮)加密;
//! 自环回( peer == local )使用 v1 以保持兼容。
//!
//! ## P0#1 fix (v1.0)
//!
//! v0.5 had a critical bug: each `Pair::new()` call generated a
//! fresh random salt and stored it in `SessionKey`.  The sender
//! then wrote its own `session.salt` into the envelope, but the
//! receiver compared `envelope.salt` against its own (different)
//! `session.salt` and rejected every message.  The fix is to make
//! the receiver ignore its cached `self.salt` and always re-derive
//! the AES key from the salt in the incoming envelope.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hkdf::Hkdf;
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tracing::instrument;
use x25519_dalek::{PublicKey, StaticSecret};

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// v1 信封的 HKDF info 字符串(单棘轮,向后兼容)。
const HKDF_INFO_V1: &[u8] = b"nebula/v0.5/e2ee";

/// v2 双棘轮根密钥 KDF info。
const KDF_RK_INFO: &[u8] = b"nebula/v1.0/dr/root";
/// v2 双棘轮链密钥 KDF info。
const KDF_CK_INFO: &[u8] = b"nebula/v1.0/dr/chain";
/// v2 初始根密钥 KDF info(从 ECDH 共享密钥派生)。
const KDF_INIT_INFO: &[u8] = b"nebula/v1.0/dr/init";
/// KDF_CK 的 IKM 常量(确保非空输入)。
const KDF_CK_IKM: &[u8] = b"nebula-dr-chain-ik";

/// v1 信封版本(单棘轮)。
pub const ENVELOPE_VERSION_V1: u8 = 1;
/// v2 信封版本(双棘轮)。
pub const ENVELOPE_VERSION_V2: u8 = 2;
/// 当前默认信封版本。跨设备新消息使用 v2。
pub const ENVELOPE_VERSION: u8 = ENVELOPE_VERSION_V2;

/// Public portion of an E2EE identity, safe to transmit across IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct E2eePublicIdentity {
    pub key_id: String,
    pub public_key_b64: String,
    pub created_at: i64,
    pub storage_type: String,
}

/// One end of a sync connection.  Cheap to clone (`StaticSecret` is
/// 32 bytes, `PublicKey` is 32 bytes).
#[derive(Clone)]
pub struct E2eeIdentity {
    pub secret: StaticSecret,
    pub public: PublicKey,
}

impl std::fmt::Debug for E2eeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("E2eeIdentity")
            .field("secret", &"<redacted>")
            .field("public", &self.public)
            .finish()
    }
}

impl PartialEq for E2eeIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.public == other.public
    }
}

impl Eq for E2eeIdentity {}

impl E2eeIdentity {
    /// Generates a fresh random X25519 identity.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Constructs an identity from existing 32-byte key material.
    /// Used when restoring from persistent storage.
    pub fn from_bytes(secret_bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(secret_bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Returns the 32-byte public key, base64-encoded.
    pub fn public_key_b64(&self) -> String {
        B64.encode(self.public.as_bytes())
    }

    /// Returns the raw secret key bytes.  This method is restricted
    /// to the backend crate; it MUST NOT be exposed through IPC.
    pub(crate) fn secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// Creates a safe-to-transmit public identity snapshot.
    pub fn to_public_identity(&self, key_id: &str, storage_type: &str) -> E2eePublicIdentity {
        E2eePublicIdentity {
            key_id: key_id.to_string(),
            public_key_b64: self.public_key_b64(),
            created_at: chrono::Utc::now().timestamp(),
            storage_type: storage_type.to_string(),
        }
    }

    /// Derives a session key with a peer's public key.
    ///
    /// The returned [`SessionKey`] stores the ECDH shared secret
    /// (symmetric for both sides) plus a per-pair random salt used
    /// by v1 encryption.  The shared secret is also used to
    /// initialise the Double Ratchet root key.
    pub fn derive_session_key(&self, peer_public: &PublicKey) -> SessionKey {
        let shared = self.secret.diffie_hellman(peer_public);
        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        SessionKey {
            shared_secret: *shared.as_bytes(),
            salt,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionKey (v1 单棘轮——向后兼容)
// ---------------------------------------------------------------------------

/// v1 会话密钥(单棘轮)。用于 v1 信封加解密和配对确认流程。
/// v2 消息加密由 [`Pair`] 内的 [`RatchetState`] 处理。
#[derive(Clone, Debug)]
pub struct SessionKey {
    /// 32-byte ECDH shared secret.  Both sides compute the same
    /// value from `(local_private, peer_public)`.
    shared_secret: [u8; 32],
    /// Cached salt used by v1 `encrypt`.  Receivers MUST NOT compare
    /// this against the envelope's salt.
    salt: [u8; 32],
}

impl SessionKey {
    /// Returns the cached salt.  Exposed for diagnostics / tests
    /// only — production code must never use this to validate
    /// an incoming envelope.
    pub fn salt(&self) -> &[u8; 32] {
        &self.salt
    }

    /// Returns the ECDH shared secret(用于双棘轮初始化)。
    pub(crate) fn shared_secret(&self) -> &[u8; 32] {
        &self.shared_secret
    }

    /// Derives the 32-byte AES-256 key for the given salt using
    /// HKDF-SHA256 over the shared secret.
    fn derive_aes_key(&self, salt: &[u8; 32]) -> [u8; 32] {
        let hk = Hkdf::<Sha256>::new(Some(salt), &self.shared_secret);
        let mut okm = [0u8; 32];
        hk.expand(HKDF_INFO_V1, &mut okm)
            .expect("32 bytes is a valid HKDF output length");
        okm
    }

    /// Encrypts a plaintext using v1 信封(配对确认用)。
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedEnvelope> {
        let key = self.derive_aes_key(&self.salt);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow!("AES-GCM encrypt failed: {e}"))?;
        Ok(EncryptedEnvelope {
            v: ENVELOPE_VERSION_V1,
            salt: self.salt.to_vec(),
            dh_pub: Vec::new(),
            n: 0,
            nonce: nonce_bytes.to_vec(),
            ciphertext: ct,
        })
    }

    /// Decrypts a v1 envelope.  P0#1 fix: the receiver ignores
    /// `self.salt` and re-derives the AES key from `envelope.salt`
    /// + the ECDH shared secret.
    pub fn decrypt(&self, envelope: &EncryptedEnvelope) -> Result<Vec<u8>> {
        if envelope.v != ENVELOPE_VERSION_V1 {
            return Err(anyhow!(
                "SessionKey only handles v1 envelopes: got v={}",
                envelope.v
            ));
        }
        if envelope.salt.len() != 32 {
            return Err(anyhow!(
                "salt must be 32 bytes, got {}",
                envelope.salt.len()
            ));
        }
        if envelope.nonce.len() != 12 {
            return Err(anyhow!("nonce must be 12 bytes"));
        }
        let mut env_salt = [0u8; 32];
        env_salt.copy_from_slice(&envelope.salt);
        let key = self.derive_aes_key(&env_salt);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let nonce = Nonce::from_slice(&envelope.nonce);
        cipher
            .decrypt(nonce, envelope.ciphertext.as_ref())
            .map_err(|e| anyhow!("AES-GCM decrypt failed: {e}"))
    }
}

/// 线缆信封。v1 使用 `salt`;v2 使用 `dh_pub` + `n`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedEnvelope {
    /// Envelope version (1 = 单棘轮, 2 = 双棘轮)。
    pub v: u8,
    /// v1: 32-byte HKDF salt。v2: 空。
    #[serde(default)]
    pub salt: Vec<u8>,
    /// v2: 32-byte DH 棘轮公钥。v1: 空。
    #[serde(default)]
    pub dh_pub: Vec<u8>,
    /// v2: 当前链消息序号。v1: 0。
    #[serde(default)]
    pub n: u32,
    /// 12-byte AES-GCM nonce。
    pub nonce: Vec<u8>,
    /// Ciphertext + 16-byte GCM tag。
    pub ciphertext: Vec<u8>,
}

impl EncryptedEnvelope {
    /// Serialises the envelope to compact JSON (base64 byte fields)。
    pub fn to_b64_json(&self) -> Result<String> {
        #[derive(Serialize)]
        struct Wire {
            v: u8,
            #[serde(skip_serializing_if = "str::is_empty")]
            salt: String,
            #[serde(skip_serializing_if = "str::is_empty")]
            dh: String,
            n: u32,
            nonce: String,
            ct: String,
        }
        let wire = Wire {
            v: self.v,
            salt: B64.encode(&self.salt),
            dh: B64.encode(&self.dh_pub),
            n: self.n,
            nonce: B64.encode(&self.nonce),
            ct: B64.encode(&self.ciphertext),
        };
        Ok(serde_json::to_string(&wire)?)
    }

    pub fn from_b64_json(s: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            v: u8,
            #[serde(default)]
            salt: String,
            #[serde(default)]
            dh: String,
            #[serde(default)]
            n: u32,
            nonce: String,
            ct: String,
        }
        let w: Wire = serde_json::from_str(s).context("parsing wire envelope")?;
        Ok(Self {
            v: w.v,
            salt: if w.salt.is_empty() {
                Vec::new()
            } else {
                B64.decode(w.salt.as_bytes()).context("decoding salt")?
            },
            dh_pub: if w.dh.is_empty() {
                Vec::new()
            } else {
                B64.decode(w.dh.as_bytes()).context("decoding dh")?
            },
            n: w.n,
            nonce: B64.decode(w.nonce.as_bytes()).context("decoding nonce")?,
            ciphertext: B64.decode(w.ct.as_bytes()).context("decoding ct")?,
        })
    }
}

// ---------------------------------------------------------------------------
// Double Ratchet 核心
// ---------------------------------------------------------------------------

/// 根密钥棘轮: `(new_rk, new_ck) = KDF_RK(rk, dh_out)`
///
/// HKDF-SHA256 with `rk` as salt, `dh_out` as IKM。
/// 输出 64 字节:前 32 = 新根密钥,后 32 = 新链密钥。
fn kdf_rk(rk: &[u8; 32], dh_out: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(Some(rk), dh_out);
    let mut okm = [0u8; 64];
    hk.expand(KDF_RK_INFO, &mut okm)
        .expect("64 bytes is a valid HKDF output length");
    let mut new_rk = [0u8; 32];
    let mut new_ck = [0u8; 32];
    new_rk.copy_from_slice(&okm[..32]);
    new_ck.copy_from_slice(&okm[32..]);
    (new_rk, new_ck)
}

/// 链密钥棘轮: `(next_ck, msg_key) = KDF_CK(ck)`
///
/// HKDF-SHA256 with `ck` as salt。单向:从 next_ck 或 msg_key
/// 无法回推 ck,实现前向保密。
fn kdf_ck(ck: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(Some(ck), KDF_CK_IKM);
    let mut okm = [0u8; 64];
    hk.expand(KDF_CK_INFO, &mut okm)
        .expect("64 bytes is a valid HKDF output length");
    let mut next_ck = [0u8; 32];
    let mut msg_key = [0u8; 32];
    next_ck.copy_from_slice(&okm[..32]);
    msg_key.copy_from_slice(&okm[32..]);
    (next_ck, msg_key)
}

/// 从 ECDH 共享密钥派生初始根密钥。
fn kdf_init_root_key(shared_secret: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);
    let mut root_key = [0u8; 32];
    hk.expand(KDF_INIT_INFO, &mut root_key)
        .expect("32 bytes is a valid HKDF output length");
    root_key
}

/// 双棘轮会话状态。初始 `dh_priv` 使用静态私钥(拷贝),
/// 首次发送时生成新 DH 密钥对,首次接收时用静态私钥做 ECDH。
#[derive(Clone)]
struct RatchetState {
    root_key: [u8; 32],
    dh_priv: StaticSecret,
    dh_pub: PublicKey,
    peer_dh_pub: PublicKey,
    send_chain_key: Option<[u8; 32]>,
    recv_chain_key: Option<[u8; 32]>,
    send_n: u32,
    recv_n: u32,
}

impl RatchetState {
    /// 从 ECDH 共享密钥和静态密钥初始化棘轮状态。
    fn new(
        shared_secret: &[u8; 32],
        local_static_secret: &StaticSecret,
        peer_static_pub: PublicKey,
    ) -> Self {
        let root_key = kdf_init_root_key(shared_secret);
        let local_static_pub = PublicKey::from(local_static_secret);
        Self {
            root_key,
            dh_priv: local_static_secret.clone(),
            dh_pub: local_static_pub,
            peer_dh_pub: peer_static_pub,
            send_chain_key: None,
            recv_chain_key: None,
            send_n: 0,
            recv_n: 0,
        }
    }

    /// 生成新 DH 密钥对。
    fn generate_dh() -> (StaticSecret, PublicKey) {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let priv_key = StaticSecret::from(bytes);
        let pub_key = PublicKey::from(&priv_key);
        (priv_key, pub_key)
    }
}

// ---------------------------------------------------------------------------
// Pair
// ---------------------------------------------------------------------------

/// One side of a paired connection.  Caches the derived session key
/// (v1) and the Double Ratchet state (v2)。
///
/// `Clone` 手动实现: `Mutex` 不实现 `Clone`,克隆时锁住内部
/// 状态并拷贝 `RatchetState`。
pub struct Pair {
    pub local: E2eeIdentity,
    pub peer_public: PublicKey,
    pub session: SessionKey,
    /// Fingerprint for human verification (truncated SHA-256 over
    /// both public keys, sorted).  Render as 6 hex groups of 4 chars.
    pub fingerprint: String,
    ratchet: Mutex<RatchetState>,
}

impl Clone for Pair {
    fn clone(&self) -> Self {
        Self {
            local: self.local.clone(),
            peer_public: self.peer_public,
            session: self.session.clone(),
            fingerprint: self.fingerprint.clone(),
            ratchet: Mutex::new(self.ratchet.lock().clone()),
        }
    }
}

impl std::fmt::Debug for Pair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pair")
            .field("local", &self.local)
            .field("peer_public", &self.peer_public)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl Pair {
    /// Establishes a pair from a local identity and the peer's
    /// 32-byte public key.  The peer key is also provided as
    /// base64 for convenience in the QR-code flow.
    pub fn new(local: E2eeIdentity, peer_public_b64: &str) -> Result<Self> {
        let peer_bytes = B64
            .decode(peer_public_b64.as_bytes())
            .context("decoding peer public key")?;
        if peer_bytes.len() != 32 {
            return Err(anyhow!(
                "peer public key must be 32 bytes, got {}",
                peer_bytes.len()
            ));
        }
        let mut peer_arr = [0u8; 32];
        peer_arr.copy_from_slice(&peer_bytes);
        let peer_public = PublicKey::from(peer_arr);
        let session = local.derive_session_key(&peer_public);
        let fingerprint = compute_fingerprint(&local.public, &peer_public);
        let ratchet = RatchetState::new(session.shared_secret(), &local.secret, peer_public);
        Ok(Self {
            local,
            peer_public,
            session,
            fingerprint,
            ratchet: Mutex::new(ratchet),
        })
    }

    /// Returns true if this is a self-pair (peer == local)。
    /// Self-pairs use v1 (SessionKey) for backward compatibility。
    fn is_self_pair(&self) -> bool {
        self.peer_public == self.local.public
    }

    /// Encrypts a plaintext.  Self-pairs use v1; cross-device pairs
    /// use v2 (Double Ratchet)。
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedEnvelope> {
        if self.is_self_pair() {
            self.session.encrypt(plaintext)
        } else {
            self.encrypt_v2(plaintext)
        }
    }

    /// Decrypts an envelope.  Dispatches by version: v1 → SessionKey,
    /// v2 → Double Ratchet。
    pub fn decrypt(&self, envelope: &EncryptedEnvelope) -> Result<Vec<u8>> {
        match envelope.v {
            ENVELOPE_VERSION_V1 => self.session.decrypt(envelope),
            ENVELOPE_VERSION_V2 => self.decrypt_v2(envelope),
            other => Err(anyhow!(
                "envelope version mismatch: got {}, expected {} or {}",
                other,
                ENVELOPE_VERSION_V1,
                ENVELOPE_VERSION_V2
            )),
        }
    }

    /// v2 加密(Double Ratchet)。在 trial clone 上操作,成功后提交。
    fn encrypt_v2(&self, plaintext: &[u8]) -> Result<EncryptedEnvelope> {
        let mut state = self.ratchet.lock();
        let mut trial = state.clone();

        // 首次发送:生成新 DH 密钥对,引导发送链
        if trial.send_chain_key.is_none() {
            let (new_priv, new_pub) = RatchetState::generate_dh();
            let dh_out = new_priv.diffie_hellman(&trial.peer_dh_pub);
            let (new_rk, new_ck) = kdf_rk(&trial.root_key, &(*dh_out.as_bytes()));
            trial.root_key = new_rk;
            trial.send_chain_key = Some(new_ck);
            trial.dh_priv = new_priv;
            trial.dh_pub = new_pub;
            trial.send_n = 0;
        }

        // 链棘轮:派生消息密钥
        let ck = trial
            .send_chain_key
            .expect("send chain key must be Some after bootstrap");
        let (next_ck, msg_key) = kdf_ck(&ck);
        trial.send_chain_key = Some(next_ck);

        // 加密
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&msg_key));
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow!("AES-GCM encrypt failed: {e}"))?;

        let envelope = EncryptedEnvelope {
            v: ENVELOPE_VERSION_V2,
            salt: Vec::new(),
            dh_pub: trial.dh_pub.as_bytes().to_vec(),
            n: trial.send_n,
            nonce: nonce_bytes.to_vec(),
            ciphertext: ct,
        };
        trial.send_n += 1;

        *state = trial;
        Ok(envelope)
    }

    /// v2 解密(Double Ratchet)。在 trial clone 上操作,
    /// AES-GCM 成功后才提交状态(失败不推进棘轮)。
    fn decrypt_v2(&self, envelope: &EncryptedEnvelope) -> Result<Vec<u8>> {
        if envelope.dh_pub.len() != 32 {
            return Err(anyhow!(
                "v2 dh_pub must be 32 bytes, got {}",
                envelope.dh_pub.len()
            ));
        }
        if envelope.nonce.len() != 12 {
            return Err(anyhow!("nonce must be 12 bytes"));
        }

        let mut peer_dh_arr = [0u8; 32];
        peer_dh_arr.copy_from_slice(&envelope.dh_pub);
        let env_peer_pub = PublicKey::from(peer_dh_arr);

        let mut state = self.ratchet.lock();
        let mut trial = state.clone();

        // DH 棘轮:收到对端新 DH 公钥时触发
        if trial.peer_dh_pub != env_peer_pub {
            trial.peer_dh_pub = env_peer_pub;

            // 步骤 1:用当前 dh_priv 推导接收链
            let dh_out = trial.dh_priv.diffie_hellman(&env_peer_pub);
            let (new_rk, new_ck) = kdf_rk(&trial.root_key, &(*dh_out.as_bytes()));
            trial.root_key = new_rk;
            trial.recv_chain_key = Some(new_ck);
            trial.recv_n = 0;

            // 步骤 2:生成新 DH 密钥对,推导发送链
            let (new_priv, new_pub) = RatchetState::generate_dh();
            let dh_out2 = new_priv.diffie_hellman(&env_peer_pub);
            let (new_rk2, new_ck2) = kdf_rk(&trial.root_key, &(*dh_out2.as_bytes()));
            trial.root_key = new_rk2;
            trial.send_chain_key = Some(new_ck2);
            trial.dh_priv = new_priv;
            trial.dh_pub = new_pub;
            trial.send_n = 0;
        }

        // 链棘轮:派生消息密钥
        let ck = trial
            .recv_chain_key
            .ok_or_else(|| anyhow!("no recv chain key — DH ratchet not initialised"))?;
        let (next_ck, msg_key) = kdf_ck(&ck);

        // 解密(trial 模式:成功才提交)
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&msg_key));
        let nonce = Nonce::from_slice(&envelope.nonce);
        match cipher.decrypt(nonce, envelope.ciphertext.as_ref()) {
            Ok(pt) => {
                trial.recv_chain_key = Some(next_ck);
                trial.recv_n += 1;
                *state = trial;
                Ok(pt)
            }
            Err(e) => Err(anyhow!("AES-GCM decrypt failed: {e}")),
        }
    }
}

/// Computes a 12-hex-char fingerprint from two public keys.  Used
/// for human-readable verification during pairing.
fn compute_fingerprint(a: &PublicKey, b: &PublicKey) -> String {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    let (lo, hi) = if a.as_bytes() < b.as_bytes() {
        (a.as_bytes(), b.as_bytes())
    } else {
        (b.as_bytes(), a.as_bytes())
    };
    hasher.update(lo);
    hasher.update(hi);
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    // First 12 chars in 3 groups of 4.
    let groups: Vec<String> = (0..3)
        .map(|i| hex[i * 4..(i + 1) * 4].to_string())
        .collect();
    groups.join("-")
}

#[instrument(skip(plaintext))]
pub fn encrypt_for_peer(
    local: &E2eeIdentity,
    peer_public_b64: &str,
    plaintext: &[u8],
) -> Result<(EncryptedEnvelope, String)> {
    let pair = Pair::new(local.clone(), peer_public_b64)?;
    let env = pair.encrypt(plaintext)?;
    Ok((env, pair.fingerprint))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // 原有测试(保持兼容)
    // -----------------------------------------------------------------------

    #[test]
    fn identity_round_trip() {
        let id = E2eeIdentity::generate();
        let pk_b64 = id.public_key_b64();
        let bytes = B64
            .decode(pk_b64.as_bytes())
            .expect("test op should succeed");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn shared_secret_matches_on_both_sides() {
        // 跨设备 Pair:alice 发首条 → bob 收(DH 棘轮)→
        // bob 发 → alice 收(DH 棘轮)。双棘轮流程。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pub = alice.public_key_b64();
        let bob_pub = bob.public_key_b64();
        let alice_pair = Pair::new(alice.clone(), &bob_pub).expect("create should succeed");
        let bob_pair = Pair::new(bob.clone(), &alice_pub).expect("create should succeed");

        // v1 SessionKey salts 仍然不同(独立随机生成)
        assert_ne!(alice_pair.session.salt(), bob_pair.session.salt());

        let env = alice_pair
            .encrypt(b"hello bob")
            .expect("test op should succeed");
        let pt = bob_pair.decrypt(&env).expect("test op should succeed");
        assert_eq!(pt, b"hello bob");

        let env2 = bob_pair
            .encrypt(b"hello alice")
            .expect("test op should succeed");
        let pt2 = alice_pair.decrypt(&env2).expect("test op should succeed");
        assert_eq!(pt2, b"hello alice");
    }

    #[test]
    fn cross_pair_decrypt_ignores_sender_salt_mismatch() {
        // 跨设备 v2:双棘轮解密,与 v1 salt 无关。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair =
            Pair::new(bob.clone(), &alice.public_key_b64()).expect("create should succeed");
        assert_ne!(alice_pair.session.salt(), bob_pair.session.salt());

        let env = alice_pair
            .encrypt(b"the quick brown fox jumps over the lazy dog")
            .expect("test op should succeed");
        let pt = bob_pair.decrypt(&env).expect("test op should succeed");
        assert_eq!(pt, b"the quick brown fox jumps over the lazy dog");
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        // 自环回( peer == local ):使用 v1 路径。
        let local = E2eeIdentity::generate();
        let peer_pub = local.public_key_b64();
        let pair = Pair::new(local, &peer_pub).expect("create should succeed");
        let plaintext = b"the quick brown fox jumps over the lazy dog";
        let env = pair.encrypt(plaintext).expect("test op should succeed");
        let pt = pair.decrypt(&env).expect("test op should succeed");
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        // 自环回 v1:篡改密文 → GCM tag 失败。
        let local = E2eeIdentity::generate();
        let pair =
            Pair::new(local.clone(), &local.public_key_b64()).expect("create should succeed");
        let mut env = pair.encrypt(b"top secret").expect("test op should succeed");
        let last = env.ciphertext.len() - 1;
        env.ciphertext[last] ^= 0x01;
        let err = pair.decrypt(&env).unwrap_err();
        assert!(err.to_string().contains("AES-GCM"));
    }

    #[test]
    fn wrong_session_key_fails() {
        // 跨设备 v2:不同 Pair 无法解密(ECDH 不匹配 → GCM 失败)。
        let a = E2eeIdentity::generate();
        let b = E2eeIdentity::generate();
        let pair_ab = Pair::new(a, &b.public_key_b64()).expect("create should succeed");
        let pair_cc = Pair::new(
            E2eeIdentity::generate(),
            &E2eeIdentity::generate().public_key_b64(),
        )
        .expect("test op should succeed");
        let env = pair_ab.encrypt(b"x").expect("test op should succeed");
        assert!(pair_cc.decrypt(&env).is_err());
    }

    #[test]
    fn tampered_salt_is_rejected() {
        // 自环回 v1:salt 篡改 → GCM tag 失败。
        let local = E2eeIdentity::generate();
        let pair =
            Pair::new(local.clone(), &local.public_key_b64()).expect("create should succeed");
        let mut env = pair.encrypt(b"salty").expect("test op should succeed");
        env.salt[0] ^= 0xff;
        let err = pair.decrypt(&env).unwrap_err();
        assert!(err.to_string().contains("AES-GCM"));
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let a = E2eeIdentity::from_bytes([1u8; 32]);
        let b = E2eeIdentity::from_bytes([2u8; 32]);
        let f1 = compute_fingerprint(&a.public, &b.public);
        let f2 = compute_fingerprint(&b.public, &a.public);
        assert_eq!(f1, f2);
        assert_eq!(f1.len(), 14); // 3 groups of 4 + 2 dashes
    }

    #[test]
    fn wire_format_round_trip() {
        // 自环回 v1:wire format 往返。
        let local = E2eeIdentity::generate();
        let pair =
            Pair::new(local.clone(), &local.public_key_b64()).expect("create should succeed");
        let env = pair
            .encrypt(b"wire-format-test")
            .expect("test op should succeed");
        let json = env.to_b64_json().expect("test op should succeed");
        let back = EncryptedEnvelope::from_b64_json(&json).expect("test op should succeed");
        let pt = pair.decrypt(&back).expect("test op should succeed");
        assert_eq!(pt, b"wire-format-test");
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let local = E2eeIdentity::generate();
        let pair =
            Pair::new(local.clone(), &local.public_key_b64()).expect("create should succeed");
        let mut env = pair.encrypt(b"v").expect("test op should succeed");
        env.v = 99;
        let err = pair.decrypt(&env).unwrap_err();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn many_messages_with_fresh_salts_all_decrypt() {
        // 跨设备 v2:alice 连续发 16 条,bob 依次解密。
        // KDF 链棘轮:每条消息唯一密钥,链顺序推进。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let pair_a = Pair::new(alice, &bob.public_key_b64()).expect("create should succeed");
        let pair_b = Pair::new(bob, &pair_a.local.public_key_b64()).expect("create should succeed");
        for i in 0..16 {
            let env = pair_a
                .encrypt(format!("message {i}").as_bytes())
                .expect("test op should succeed");
            let pt = pair_b.decrypt(&env).expect("test op should succeed");
            assert_eq!(pt, format!("message {i}").as_bytes());
        }
    }

    // -----------------------------------------------------------------------
    // Double Ratchet 新增测试
    // -----------------------------------------------------------------------

    #[test]
    fn double_ratchet_bidirectional_conversation() {
        // 多轮双向对话:alice ↔ bob 交替发送,全部解密成功。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair =
            Pair::new(bob.clone(), &alice.public_key_b64()).expect("create should succeed");

        // Alice → Bob
        for i in 0..5 {
            let msg = format!("alice says {i}");
            let env = alice_pair.encrypt(msg.as_bytes()).expect("encrypt");
            let pt = bob_pair.decrypt(&env).expect("decrypt");
            assert_eq!(pt, msg.as_bytes());
        }
        // Bob → Alice
        for i in 0..5 {
            let msg = format!("bob says {i}");
            let env = bob_pair.encrypt(msg.as_bytes()).expect("encrypt");
            let pt = alice_pair.decrypt(&env).expect("decrypt");
            assert_eq!(pt, msg.as_bytes());
        }
        // Alice → Bob (again, after DH ratchet)
        for i in 0..3 {
            let msg = format!("alice again {i}");
            let env = alice_pair.encrypt(msg.as_bytes()).expect("encrypt");
            let pt = bob_pair.decrypt(&env).expect("decrypt");
            assert_eq!(pt, msg.as_bytes());
        }
    }

    #[test]
    fn dh_ratchet_changes_dh_pubkey() {
        // DH 棘轮:对话方向切换时,DH 公钥更新。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair =
            Pair::new(bob.clone(), &alice.public_key_b64()).expect("create should succeed");

        // Alice 首条:DH pub A1
        let env1 = alice_pair.encrypt(b"msg1").expect("encrypt");
        let dh1 = env1.dh_pub.clone();
        assert_eq!(dh1.len(), 32);
        bob_pair.decrypt(&env1).expect("decrypt");

        // Alice 第二条:同 DH pub(同一链)
        let env2 = alice_pair.encrypt(b"msg2").expect("encrypt");
        let dh2 = env2.dh_pub.clone();
        assert_eq!(dh1, dh2, "same DH pub within one chain");
        bob_pair.decrypt(&env2).expect("decrypt");

        // Bob 发回:新 DH pub B1(不同链)
        let env3 = bob_pair.encrypt(b"msg3").expect("encrypt");
        let dh3 = env3.dh_pub.clone();
        assert_ne!(dh1, dh3, "DH pub changes on direction switch");
        alice_pair.decrypt(&env3).expect("decrypt");
    }

    #[test]
    fn message_keys_are_unique() {
        // 每条消息的密文不同(唯一消息密钥 + 随机 nonce)。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair = Pair::new(alice, &bob.public_key_b64()).expect("create should succeed");

        let mut ciphertexts = Vec::new();
        for i in 0..10 {
            let env = alice_pair
                .encrypt(format!("msg{i}").as_bytes())
                .expect("encrypt");
            ciphertexts.push(env.ciphertext);
        }
        // 所有密文互不相同
        for i in 0..ciphertexts.len() {
            for j in (i + 1)..ciphertexts.len() {
                assert_ne!(
                    ciphertexts[i], ciphertexts[j],
                    "ciphertexts {i} and {j} must differ"
                );
            }
        }
    }

    #[test]
    fn forward_secrecy_old_messages_undecryptable_after_ratchet() {
        // 前向保密:DH 棘轮后,旧链消息无法用新状态解密。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair = Pair::new(bob, &alice.public_key_b64()).expect("create should succeed");
        let env1 = alice_pair.encrypt(b"old message").expect("encrypt");
        bob_pair.decrypt(&env1).expect("decrypt ok");

        // Bob 发回 msg2(alice 收,DH 棘轮)
        let env2 = bob_pair.encrypt(b"trigger ratchet").expect("encrypt");
        alice_pair.decrypt(&env2).expect("decrypt ok");

        // Alice 再发 msg3(bob 收,又一次 DH 棘轮)
        let env3 = alice_pair.encrypt(b"new chain").expect("encrypt");
        bob_pair.decrypt(&env3).expect("decrypt ok");

        // 尝试用 bob 当前状态解密 msg1(旧链消息)
        // bob 的 peer_dh_pub 现在是 env3 的 DH pub,与 env1 不同 →
        // DH 棘轮会使用不同的 dh_priv,ECDH 不匹配 → GCM 失败。
        // (trial 模式:失败不提交,不影响后续正常使用)
        let result = bob_pair.decrypt(&env1);
        assert!(
            result.is_err(),
            "old message should NOT be decryptable after DH ratchet (forward secrecy)"
        );
    }

    #[test]
    fn forward_secrecy_chain_advanced_old_message_fails() {
        // 前向保密:链推进后,旧消息无法用推进后的链解密。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair = Pair::new(bob, &alice.public_key_b64()).expect("create should succeed");

        // Alice 发 3 条,bob 依次收(链推进 3 次)
        let envs: Vec<_> = (0..3)
            .map(|i| {
                alice_pair
                    .encrypt(format!("msg{i}").as_bytes())
                    .expect("encrypt")
            })
            .collect();
        for env in &envs {
            bob_pair.decrypt(env).expect("decrypt");
        }

        // 尝试用 bob 推进后的状态解密 envs[0](旧链位置 0)
        // bob 的 recv_chain 已推进到位置 3,envs[0] 在位置 0。
        // 同一 DH pub → 不触发 DH 棘轮,但 KDF_CK 推进后
        // 消息密钥不匹配 → GCM 失败。
        let result = bob_pair.decrypt(&envs[0]);
        assert!(
            result.is_err(),
            "old chain message should fail after chain advance (forward secrecy)"
        );
    }

    #[test]
    fn v1_envelope_backward_compatible() {
        // v1 信封(SessionKey 路径)仍可被 Pair::decrypt 解密。
        let local = E2eeIdentity::generate();
        let pair =
            Pair::new(local.clone(), &local.public_key_b64()).expect("create should succeed");

        // 直接用 SessionKey 生成 v1 信封
        let env = pair.session.encrypt(b"v1 message").expect("encrypt");
        assert_eq!(env.v, ENVELOPE_VERSION_V1);
        assert!(!env.salt.is_empty());
        assert!(env.dh_pub.is_empty());

        // Pair::decrypt 能处理 v1
        let pt = pair.decrypt(&env).expect("decrypt");
        assert_eq!(pt, b"v1 message");
    }

    #[test]
    fn v1_v2_wire_format_compatible() {
        // v1 和 v2 信封 wire format 往返。
        let local = E2eeIdentity::generate();
        let pair =
            Pair::new(local.clone(), &local.public_key_b64()).expect("create should succeed");

        // v1 信封 wire 往返
        let env1 = pair.session.encrypt(b"v1").expect("encrypt");
        let json1 = env1.to_b64_json().expect("serialize");
        let back1 = EncryptedEnvelope::from_b64_json(&json1).expect("deserialize");
        assert_eq!(back1.v, ENVELOPE_VERSION_V1);
        assert_eq!(back1.salt, env1.salt);
        assert!(back1.dh_pub.is_empty());
        pair.decrypt(&back1).expect("v1 decrypt");

        // 跨设备 v2 信封 wire 往返
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair =
            Pair::new(bob.clone(), &alice.public_key_b64()).expect("create should succeed");

        let env2 = alice_pair.encrypt(b"v2").expect("encrypt");
        assert_eq!(env2.v, ENVELOPE_VERSION_V2);
        assert!(env2.salt.is_empty());
        assert_eq!(env2.dh_pub.len(), 32);

        let json2 = env2.to_b64_json().expect("serialize");
        let back2 = EncryptedEnvelope::from_b64_json(&json2).expect("deserialize");
        assert_eq!(back2, env2);
        let pt = bob_pair.decrypt(&back2).expect("v2 decrypt");
        assert_eq!(pt, b"v2");
    }

    #[test]
    fn failed_decrypt_preserves_state() {
        // 解密失败不推进棘轮:失败后正常消息仍可解密。
        // trial clone 模式: AES-GCM 成功后才提交 state,失败回滚。
        let alice = E2eeIdentity::generate();
        let bob = E2eeIdentity::generate();
        let alice_pair =
            Pair::new(alice.clone(), &bob.public_key_b64()).expect("create should succeed");
        let bob_pair = Pair::new(bob, &alice.public_key_b64()).expect("create should succeed");

        // Alice 发 msg1
        let env1 = alice_pair.encrypt(b"first message").expect("encrypt");

        // 篡改 env1 密文,使 bob 解密失败
        let mut tampered = env1.clone();
        let last = tampered.ciphertext.len() - 1;
        tampered.ciphertext[last] ^= 0xff;
        let err = bob_pair.decrypt(&tampered);
        assert!(err.is_err(), "tampered message must fail");

        // bob 状态未推进:原始 env1 仍可解密
        let pt = bob_pair
            .decrypt(&env1)
            .expect("decrypt original after failed attempt");
        assert_eq!(pt, b"first message");

        // Alice 发第二条,bob 仍可正常解密(链棘轮顺序正确)
        let env2 = alice_pair.encrypt(b"second message").expect("encrypt");
        let pt2 = bob_pair.decrypt(&env2).expect("decrypt second");
        assert_eq!(pt2, b"second message");
    }

    #[test]
    fn envelope_version_constants() {
        // 版本常量正确性。
        assert_eq!(ENVELOPE_VERSION_V1, 1);
        assert_eq!(ENVELOPE_VERSION_V2, 2);
        assert_eq!(ENVELOPE_VERSION, ENVELOPE_VERSION_V2);
    }
}
