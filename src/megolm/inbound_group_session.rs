// Copyright 2021 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::io::{Cursor, Read};

use block_modes::BlockModeError;
use ed25519_dalek::{
    PublicKey, Signature, SignatureError, Verifier, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH,
};
use hmac::digest::MacError;
use thiserror::Error;

use super::{message::MegolmMessage, ratchet::Ratchet, SESSION_KEY_VERSION};
use crate::{cipher::Cipher, messages::DecodeError, utilities::base64_decode};

#[derive(Debug, Error)]
pub enum SessoinCreationError {
    #[error("The session had a invalid version, expected {0}, got {1}")]
    Version(u8, u8),
    #[error("The session key was too short {0}")]
    Read(#[from] std::io::Error),
    #[error("The session key wasn't valid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("The signature on the session key was invalid: {0}")]
    Signature(#[from] SignatureError),
}

#[derive(Debug, Error)]
pub enum DecryptionError {
    #[error("The message wasn't valid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("The signature on the session key was invalid: {0}")]
    Signature(#[from] SignatureError),
    #[error("Failed decrypting Megolm message, invalid MAC: {0}")]
    InvalidMAC(#[from] MacError),
    #[error("Failed decrypting Megolm message, invalid ciphertext: {0}")]
    InvalidCiphertext(#[from] BlockModeError),
    #[error(
        "The message was encrypted using an unknown message index, \
        first known index {0}, index of the message {1}"
    )]
    UnknownMessageIndex(u32, u32),
    #[error("The message couldn't be decoded: {0}")]
    DecodeError(#[from] DecodeError),
}

pub struct InboundGroupSession {
    initial_ratchet: Ratchet,
    latest_ratchet: Ratchet,
    signing_key: PublicKey,
}

pub struct DecryptedMessage {
    pub plaintext: String,
    pub message_index: u32,
}

impl InboundGroupSession {
    pub fn new(session_key: String) -> Result<Self, SessoinCreationError> {
        let decoded = base64_decode(session_key)?;
        let mut cursor = Cursor::new(decoded);

        let mut version = [0u8; 1];
        let mut index = [0u8; 4];
        let mut ratchet = [0u8; 128];
        let mut public_key = [0u8; PUBLIC_KEY_LENGTH];
        let mut signature = [0u8; SIGNATURE_LENGTH];

        cursor.read_exact(&mut version)?;

        if version[0] != SESSION_KEY_VERSION {
            Err(SessoinCreationError::Version(SESSION_KEY_VERSION, version[0]))
        } else {
            cursor.read_exact(&mut index)?;
            cursor.read_exact(&mut ratchet)?;
            cursor.read_exact(&mut public_key)?;
            cursor.read_exact(&mut signature)?;

            let index = u32::from_le_bytes(index);
            let initial_ratchet = Ratchet::from_bytes(ratchet, index);
            let latest_ratchet = initial_ratchet.clone();

            let signing_key = PublicKey::from_bytes(&public_key)?;
            let signature = Signature::from_bytes(&signature)?;

            let decoded = cursor.into_inner();

            signing_key.verify(&decoded[..decoded.len() - 64], &signature)?;

            Ok(Self { initial_ratchet, latest_ratchet, signing_key })
        }
    }

    fn find_ratchet(&mut self, message_index: u32) -> Option<&Ratchet> {
        if self.initial_ratchet.index() == message_index {
            Some(&self.initial_ratchet)
        } else if self.latest_ratchet.index() == message_index {
            Some(&self.latest_ratchet)
        } else if self.latest_ratchet.index() < message_index {
            self.latest_ratchet.advance_to(message_index);
            Some(&self.latest_ratchet)
        } else if self.initial_ratchet.index() < message_index {
            self.latest_ratchet = self.initial_ratchet.clone();
            self.latest_ratchet.advance_to(message_index);
            Some(&self.latest_ratchet)
        } else {
            None
        }
    }

    pub fn decrypt(&mut self, ciphertext: &str) -> Result<DecryptedMessage, DecryptionError> {
        let decoded = base64_decode(ciphertext)?;
        let (message, decoded) = MegolmMessage::decode(decoded)?;

        self.signing_key.verify(message.bytes_for_signing(), &decoded.signature)?;

        if let Some(ratchet) = self.find_ratchet(decoded.message_index) {
            let cipher = Cipher::new_megolm(ratchet.as_bytes());

            cipher.verify_mac(message.bytes_for_mac(), &decoded.mac)?;
            let plaintext =
                String::from_utf8_lossy(&cipher.decrypt(&decoded.ciphertext)?).to_string();

            Ok(DecryptedMessage { plaintext, message_index: decoded.message_index })
        } else {
            Err(DecryptionError::UnknownMessageIndex(
                self.initial_ratchet.index(),
                decoded.message_index,
            ))
        }
    }

    pub fn export_at(&mut self) -> String {
        todo!()
    }
}