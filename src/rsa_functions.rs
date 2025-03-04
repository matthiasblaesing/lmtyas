use crate::base64_trait::{Base64StringConversions, Base64VecU8Conversions};
use crate::unsecure_string::SecureStringToUnsecureString;
use log::{debug, warn , info};
use openssl::rsa::{Padding, Rsa};
use secstr::SecStr;
use serde::Deserialize;
use std::error::Error;
use std::path::Path;
use zeroize::Zeroize;

// min bit size of the modulus (modulus * 8 = rsa key bits)
const MIN_RSA_MODULUS_SIZE: u32 = 256;
/// Holds the RSA private and public for
/// encryption and decryption
pub struct RsaKeys {
    // Option<> to construct empty values
    pub rsa_private_key: Option<Rsa<openssl::pkey::Private>>,
    // Option<> to construct empty values
    pub rsa_public_key: Option<Rsa<openssl::pkey::Public>>,
}

impl Default for RsaKeys {
    fn default() -> Self {
        Self::new()
    }
}

impl RsaKeys {
    /// Constructs data struct with
    /// None<> to be able to put it into a
    /// arc<RwLock<Rsakeys>>
    ///
    /// # Returns
    ///
    /// - RsaKeys{rsa_private_key: None, rsa_public_key: None,}
    pub fn new() -> RsaKeys {
        RsaKeys {
            rsa_private_key: None,
            rsa_public_key: None,
        }
    }
    /// Loads RSA private and public key from the given paths.
    /// To load the RSA privte key, the passphrase is needed.
    ///
    /// # Arguments
    ///
    /// - rsa_private_key_path: Path
    /// - rsa_public_key_path:  Path
    /// - secure_passphrase:    SecStr
    ///
    /// # Returns
    ///
    /// - Result<RsaKeys, Box<dyn Error>>
    pub fn read_from_files<P: AsRef<Path>>(
        &mut self,
        rsa_private_key_path: P,
        rsa_public_key_path: P,
        secure_passphrase: &SecStr,
    ) -> Result<(), Box<dyn Error>> {
        let rsa_private_key_file = std::fs::read_to_string(rsa_private_key_path)?;
        let mut unsecure_passphrase = secure_passphrase.to_unsecure_string();
        let rsa_private_key = match Rsa::private_key_from_pem_passphrase(
            rsa_private_key_file.as_bytes(),
            unsecure_passphrase.as_bytes(),
        ) {
            Ok(p) => p,
            Err(e) => {
                unsecure_passphrase.zeroize();
                warn!("cannot load rsa private key: {}", e);
                const RSA_CANNOT_LOAD_KEY: &str = "Cannot load rsa keys!";
                let boxed_error = Box::<dyn Error + Send + Sync>::from(RSA_CANNOT_LOAD_KEY);
                return Err(boxed_error);
            }
        };
        unsecure_passphrase.zeroize();
        let rsa_public_key_file = std::fs::read_to_string(rsa_public_key_path)?;
        let rsa_public_key = Rsa::public_key_from_pem(rsa_public_key_file.as_bytes())?;
        debug!("rsa_public_key.size() = {}", &rsa_public_key.size());
        if rsa_public_key.size() < MIN_RSA_MODULUS_SIZE {
            warn!("modulus is < {} bytes", MIN_RSA_MODULUS_SIZE);
            const RSA_MIN_MODULUS_ERR: &str = "RSA key size too small";
            let boxed_error = Box::<dyn Error + Send + Sync>::from(RSA_MIN_MODULUS_ERR);
            return Err(boxed_error);
        }
        self.rsa_private_key = Some(rsa_private_key);
        self.rsa_public_key = Some(rsa_public_key);
        Ok(())
    }

    /// Encrypt a String slice with stored RSA public key
    /// and return it as base64 encoded String.
    ///
    /// # Arguments
    ///
    /// - `plaintext_data`: a String slice with data to encrypt
    pub fn encrypt_str(&self, plaintext_data: &str) -> Result<String, Box<dyn Error>> {
        if self.rsa_public_key.is_none() {
            let box_err: Box<dyn Error> = "RSA public key is not set!".to_string().into();
            return Err(box_err);
        }
        let public_key = self.rsa_public_key.as_ref().unwrap();
        let mut buf: Vec<u8> = vec![0; public_key.size() as usize];
        match public_key.public_encrypt(plaintext_data.as_bytes(), &mut buf, Padding::PKCS1) {
            Err(e) => {
                println!("Could not rsa encrypt given value: {}", &e);
                info!("Could not rsa encrypt given value: {}", &e);
                let box_err: Box<dyn Error> =
                    "Could not rsa encrypt given value".to_string().into();
                Err(box_err)
            }
            Ok(_) => {
                let base64_encrypted = buf.to_base64_encoded();
                Ok(base64_encrypted)
            }
        }
    }

    /// Decrypt a base64 encoded String slice with stored RSA private key
    /// and return it as plaintext String.
    ///
    /// # Arguments
    ///
    /// - `encrypted_data`: a String slice with data to decrypt
    pub fn decrypt_str(&self, encrypted_data: &str) -> Result<String, Box<dyn Error>> {
        if self.rsa_private_key.is_none() {
            let box_err: Box<dyn Error> = "RSA private key is not set!".to_string().into();
            return Err(box_err);
        }
        let raw_data = match Vec::from_base64_encoded(encrypted_data) {
            Ok(b) => b,
            Err(e) => {
                println!("Could not base64 decode given value: {}", &e);
                warn!(
                    "decrypt_str() => could not base64 decode given value: {}",
                    &e
                );
                let box_err: Box<dyn Error> =
                    "Could not base64 decode given value".to_string().into();
                return Err(box_err);
            }
        };

        let private_key = self.rsa_private_key.as_ref().unwrap();
        let mut buf: Vec<u8> = vec![0; private_key.size() as usize];
        match private_key.private_decrypt(&raw_data, &mut buf, Padding::PKCS1) {
            Err(e) => {
                println!("Could not rsa decrypt given value: {}", &e);
                info!("Could not rsa decrypt given value: {}", &e);
                let box_err: Box<dyn Error> =
                    "Could not rsa decrypt given value".to_string().into();
                Err(box_err)
            }
            Ok(_) => {
                let decrypted_data = match String::from_utf8(buf) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("Could not convert decrypted data to utf8: {}", &e);
                        info!("Could not convert decrypted data to utf8: {}", &e);
                        let box_err: Box<dyn Error> = "Could not convert decrypted data to utf8"
                            .to_string()
                            .into();
                        return Err(box_err);
                    }
                };
                Ok(decrypted_data.trim_matches(char::from(0)).to_string())
            }
        }
    }
}

/// Holds the password for the RSA private key
/// that encrypts secrets and links.
#[derive(Clone, Deserialize, Debug)]
pub struct RsaPrivateKeyPassword {
    pub rsa_private_key_password: Option<SecStr>,
}
