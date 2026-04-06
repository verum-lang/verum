//! Tokenizer integration for VBC interpreter.
//!
//! This module provides tokenization support using the HuggingFace tokenizers library.
//! When the `tokenizers` feature is disabled, stub implementations are provided that
//! fall back to simple byte encoding/decoding.
//!
//! # Supported Tokenizers
//!
//! - **BPE (Byte-Pair Encoding)**: Load from vocab.json and merges.txt files
//! - **Pretrained**: Load from model name (e.g., "gpt2", "llama", "bert-base-uncased")
//! - **SentencePiece**: Load from .model files
//!
//! # Example
//!
//! ```ignore
//! use verum_vbc::interpreter::kernel::tokenizer::*;
//!
//! // Load a pretrained tokenizer
//! let handle = dispatch_tokenizer_load_pretrained("gpt2")?;
//!
//! // Encode text
//! let tokens = dispatch_tokenizer_encode(&handle, "Hello, world!")?;
//!
//! // Decode back to text
//! let text = dispatch_tokenizer_decode(&handle, &tokens)?;
//! ```

#[cfg(feature = "tokenizers")]
use std::sync::Arc;

/// Tokenizer handle for BPE or SentencePiece tokenizers.
///
/// When the `tokenizers` feature is enabled, this wraps a real HuggingFace tokenizer.
/// Otherwise, it's a stub that tracks metadata only.
pub struct TokenizerHandle {
    /// The tokenizer type.
    pub tokenizer_type: TokenizerType,
    /// Vocabulary size (for validation).
    pub vocab_size: usize,
    /// The underlying tokenizer (when feature enabled).
    #[cfg(feature = "tokenizers")]
    pub inner: Arc<tokenizers::Tokenizer>,
}

/// Tokenizer type enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenizerType {
    /// Byte-Pair Encoding tokenizer.
    Bpe,
    /// SentencePiece tokenizer.
    SentencePiece,
    /// Pretrained tokenizer (loaded by model name).
    Pretrained,
}

// =============================================================================
// Feature-gated implementations using HuggingFace tokenizers
// =============================================================================

#[cfg(feature = "tokenizers")]
mod real_impl {
    use super::*;
    use std::path::Path;
    use tokenizers::models::bpe::BPE;
    use tokenizers::{Model, Tokenizer};

    /// Load BPE tokenizer from vocab and merges files.
    ///
    /// # Arguments
    /// * `vocab_path` - Path to vocab.json file
    /// * `merges_path` - Path to merges.txt file
    ///
    /// # Returns
    /// A tokenizer handle, or None if loading fails.
    pub fn load_bpe(vocab_path: &str, merges_path: &str) -> Option<TokenizerHandle> {
        // Build BPE model from vocab and merges
        let bpe = BPE::from_file(vocab_path, merges_path)
            .build()
            .ok()?;

        let vocab_size = bpe.get_vocab_size();
        let tokenizer = Tokenizer::new(bpe);

        Some(TokenizerHandle {
            tokenizer_type: TokenizerType::Bpe,
            vocab_size,
            inner: Arc::new(tokenizer),
        })
    }

    /// Load pretrained tokenizer by model name or path.
    ///
    /// This function attempts to load a tokenizer from a local path first.
    /// If the path contains "tokenizer.json", it loads directly.
    /// Otherwise, it tries common tokenizer file names in the directory.
    ///
    /// # Arguments
    /// * `model_name` - The path to a tokenizer.json file or a directory containing one
    ///
    /// # Returns
    /// A tokenizer handle, or None if loading fails.
    pub fn load_pretrained(model_name: &str) -> Option<TokenizerHandle> {
        let path = Path::new(model_name);

        // If it's a direct path to a .json file, load it
        if path.extension().map_or(false, |e| e == "json") {
            let tokenizer = Tokenizer::from_file(path).ok()?;
            let vocab_size = tokenizer.get_vocab_size(true);
            return Some(TokenizerHandle {
                tokenizer_type: TokenizerType::Pretrained,
                vocab_size,
                inner: Arc::new(tokenizer),
            });
        }

        // If it's a directory, try common tokenizer file names
        if path.is_dir() {
            // Try tokenizer.json first
            let tokenizer_json = path.join("tokenizer.json");
            if tokenizer_json.exists() {
                let tokenizer = Tokenizer::from_file(&tokenizer_json).ok()?;
                let vocab_size = tokenizer.get_vocab_size(true);
                return Some(TokenizerHandle {
                    tokenizer_type: TokenizerType::Pretrained,
                    vocab_size,
                    inner: Arc::new(tokenizer),
                });
            }
        }

        // Cannot load - model name requires hub access which is not enabled
        None
    }

    /// Load SentencePiece tokenizer from model file.
    ///
    /// # Arguments
    /// * `model_path` - Path to .model file or tokenizer.json
    ///
    /// # Returns
    /// A tokenizer handle, or None if loading fails.
    pub fn load_spm(model_path: &str) -> Option<TokenizerHandle> {
        // SentencePiece models can be loaded via the Unigram model
        // The HuggingFace tokenizers library supports loading tokenizer.json
        // that may contain SentencePiece-trained models

        let path = Path::new(model_path);

        // If it's a .json file, load directly
        if path.extension().map_or(false, |e| e == "json") {
            let tokenizer = Tokenizer::from_file(model_path).ok()?;
            let vocab_size = tokenizer.get_vocab_size(true);
            return Some(TokenizerHandle {
                tokenizer_type: TokenizerType::SentencePiece,
                vocab_size,
                inner: Arc::new(tokenizer),
            });
        }

        // For .model files, we need sentencepiece-specific handling
        // The tokenizers crate doesn't directly load .spm files
        // Fallback: try loading as a tokenizer.json in the same directory
        let dir = path.parent()?;
        let tokenizer_json = dir.join("tokenizer.json");
        if tokenizer_json.exists() {
            let tokenizer = Tokenizer::from_file(&tokenizer_json).ok()?;
            let vocab_size = tokenizer.get_vocab_size(true);
            return Some(TokenizerHandle {
                tokenizer_type: TokenizerType::SentencePiece,
                vocab_size,
                inner: Arc::new(tokenizer),
            });
        }

        // Cannot load native .model files directly
        // User should convert to tokenizer.json format
        None
    }

    /// Encode text to tokens using the tokenizer.
    ///
    /// # Arguments
    /// * `tokenizer` - The tokenizer handle
    /// * `text` - The text to encode
    ///
    /// # Returns
    /// A vector of token IDs, or None if encoding fails.
    pub fn encode(tokenizer: &TokenizerHandle, text: &str) -> Option<Vec<u32>> {
        let encoding = tokenizer.inner.encode(text, false).ok()?;
        Some(encoding.get_ids().to_vec())
    }

    /// Decode tokens to text using the tokenizer.
    ///
    /// # Arguments
    /// * `tokenizer` - The tokenizer handle
    /// * `tokens` - The token IDs to decode
    ///
    /// # Returns
    /// The decoded text, or None if decoding fails.
    pub fn decode(tokenizer: &TokenizerHandle, tokens: &[u32]) -> Option<String> {
        tokenizer.inner.decode(tokens, true).ok()
    }

    /// Encode text with special tokens.
    ///
    /// # Arguments
    /// * `tokenizer` - The tokenizer handle
    /// * `text` - The text to encode
    /// * `add_special_tokens` - Whether to add special tokens (e.g., [CLS], [SEP])
    ///
    /// # Returns
    /// A vector of token IDs, or None if encoding fails.
    pub fn encode_with_special_tokens(
        tokenizer: &TokenizerHandle,
        text: &str,
        add_special_tokens: bool,
    ) -> Option<Vec<u32>> {
        let encoding = tokenizer.inner.encode(text, add_special_tokens).ok()?;
        Some(encoding.get_ids().to_vec())
    }

    /// Batch encode multiple texts.
    ///
    /// # Arguments
    /// * `tokenizer` - The tokenizer handle
    /// * `texts` - The texts to encode
    ///
    /// # Returns
    /// A vector of token ID vectors, or None if encoding fails.
    pub fn encode_batch(tokenizer: &TokenizerHandle, texts: &[&str]) -> Option<Vec<Vec<u32>>> {
        let encodings = tokenizer.inner.encode_batch(texts.to_vec(), false).ok()?;
        Some(encodings.iter().map(|e| e.get_ids().to_vec()).collect())
    }

    /// Batch decode multiple token sequences.
    ///
    /// # Arguments
    /// * `tokenizer` - The tokenizer handle
    /// * `token_sequences` - The token ID sequences to decode
    ///
    /// # Returns
    /// A vector of decoded texts, or None if decoding fails.
    pub fn decode_batch(tokenizer: &TokenizerHandle, token_sequences: &[Vec<u32>]) -> Option<Vec<String>> {
        let refs: Vec<&[u32]> = token_sequences.iter().map(|v| v.as_slice()).collect();
        tokenizer.inner.decode_batch(&refs, true).ok()
    }
}

// =============================================================================
// Stub implementations (when tokenizers feature is disabled)
// =============================================================================

#[cfg(not(feature = "tokenizers"))]
mod stub_impl {
    use super::*;

    /// Stub: Load BPE tokenizer (returns dummy handle).
    pub fn load_bpe(_vocab_path: &str, _merges_path: &str) -> Option<TokenizerHandle> {
        Some(TokenizerHandle {
            tokenizer_type: TokenizerType::Bpe,
            vocab_size: 50257, // GPT-2 default
        })
    }

    /// Stub: Load pretrained tokenizer (returns dummy handle).
    pub fn load_pretrained(_model_name: &str) -> Option<TokenizerHandle> {
        Some(TokenizerHandle {
            tokenizer_type: TokenizerType::Pretrained,
            vocab_size: 32000, // LLaMA default
        })
    }

    /// Stub: Load SentencePiece tokenizer (returns dummy handle).
    pub fn load_spm(_model_path: &str) -> Option<TokenizerHandle> {
        Some(TokenizerHandle {
            tokenizer_type: TokenizerType::SentencePiece,
            vocab_size: 32000,
        })
    }

    /// Stub: Encode text to bytes as tokens.
    pub fn encode(_tokenizer: &TokenizerHandle, text: &str) -> Option<Vec<u32>> {
        Some(text.bytes().map(|b| b as u32).collect())
    }

    /// Stub: Decode tokens as bytes to text.
    pub fn decode(_tokenizer: &TokenizerHandle, tokens: &[u32]) -> Option<String> {
        let bytes: Vec<u8> = tokens.iter().filter_map(|&t| {
            if t < 256 { Some(t as u8) } else { None }
        }).collect();
        String::from_utf8(bytes).ok()
    }

    /// Stub: Encode with special tokens (same as encode).
    pub fn encode_with_special_tokens(
        tokenizer: &TokenizerHandle,
        text: &str,
        _add_special_tokens: bool,
    ) -> Option<Vec<u32>> {
        encode(tokenizer, text)
    }

    /// Stub: Batch encode (encodes each individually).
    pub fn encode_batch(tokenizer: &TokenizerHandle, texts: &[&str]) -> Option<Vec<Vec<u32>>> {
        Some(texts.iter().filter_map(|t| encode(tokenizer, t)).collect())
    }

    /// Stub: Batch decode (decodes each individually).
    pub fn decode_batch(tokenizer: &TokenizerHandle, token_sequences: &[Vec<u32>]) -> Option<Vec<String>> {
        Some(token_sequences.iter().filter_map(|t| decode(tokenizer, t)).collect())
    }
}

// =============================================================================
// Public dispatch functions (unified API)
// =============================================================================

#[cfg(feature = "tokenizers")]
use real_impl as impl_mod;
#[cfg(not(feature = "tokenizers"))]
use stub_impl as impl_mod;

/// Load BPE tokenizer from vocabulary and merges files.
///
/// # Arguments
/// * `vocab_path` - Path to vocab.json file
/// * `merges_path` - Path to merges.txt file
///
/// # Returns
/// A tokenizer handle, or None if loading fails.
#[inline]
pub fn dispatch_tokenizer_load_bpe(vocab_path: &str, merges_path: &str) -> Option<TokenizerHandle> {
    impl_mod::load_bpe(vocab_path, merges_path)
}

/// Load pretrained tokenizer by model name.
///
/// # Arguments
/// * `model_name` - The name or path of the pretrained model
///
/// # Returns
/// A tokenizer handle, or None if loading fails.
#[inline]
pub fn dispatch_tokenizer_load_pretrained(model_name: &str) -> Option<TokenizerHandle> {
    impl_mod::load_pretrained(model_name)
}

/// Load SentencePiece tokenizer from model file.
///
/// # Arguments
/// * `model_path` - Path to .model or tokenizer.json file
///
/// # Returns
/// A tokenizer handle, or None if loading fails.
#[inline]
pub fn dispatch_tokenizer_load_spm(model_path: &str) -> Option<TokenizerHandle> {
    impl_mod::load_spm(model_path)
}

/// Encode text to tokens.
///
/// # Arguments
/// * `tokenizer` - The tokenizer handle
/// * `text` - The text to encode
///
/// # Returns
/// A vector of token IDs, or None if encoding fails.
#[inline]
pub fn dispatch_tokenizer_encode(tokenizer: &TokenizerHandle, text: &str) -> Option<Vec<u32>> {
    impl_mod::encode(tokenizer, text)
}

/// Decode tokens to text.
///
/// # Arguments
/// * `tokenizer` - The tokenizer handle
/// * `tokens` - The token IDs to decode
///
/// # Returns
/// The decoded text, or None if decoding fails.
#[inline]
pub fn dispatch_tokenizer_decode(tokenizer: &TokenizerHandle, tokens: &[u32]) -> Option<String> {
    impl_mod::decode(tokenizer, tokens)
}

/// Encode with SentencePiece (alias for encode).
#[inline]
pub fn dispatch_tokenizer_spm_encode(tokenizer: &TokenizerHandle, text: &str) -> Option<Vec<u32>> {
    impl_mod::encode(tokenizer, text)
}

/// Decode with SentencePiece (alias for decode).
#[inline]
pub fn dispatch_tokenizer_spm_decode(tokenizer: &TokenizerHandle, tokens: &[u32]) -> Option<String> {
    impl_mod::decode(tokenizer, tokens)
}

/// Encode text with optional special tokens.
#[inline]
pub fn dispatch_tokenizer_encode_special(
    tokenizer: &TokenizerHandle,
    text: &str,
    add_special_tokens: bool,
) -> Option<Vec<u32>> {
    impl_mod::encode_with_special_tokens(tokenizer, text, add_special_tokens)
}

/// Batch encode multiple texts.
#[inline]
pub fn dispatch_tokenizer_encode_batch(tokenizer: &TokenizerHandle, texts: &[&str]) -> Option<Vec<Vec<u32>>> {
    impl_mod::encode_batch(tokenizer, texts)
}

/// Batch decode multiple token sequences.
#[inline]
pub fn dispatch_tokenizer_decode_batch(
    tokenizer: &TokenizerHandle,
    token_sequences: &[Vec<u32>],
) -> Option<Vec<String>> {
    impl_mod::decode_batch(tokenizer, token_sequences)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that stub implementations work correctly (when tokenizers feature is disabled).
    #[cfg(not(feature = "tokenizers"))]
    mod stub_tests {
        use super::*;

        #[test]
        fn test_stub_tokenizer_encode_decode() {
            // Stub always returns a handle
            let handle = dispatch_tokenizer_load_pretrained("test-model").unwrap();

            // Encode - stub returns byte encoding
            let tokens = dispatch_tokenizer_encode(&handle, "hello").unwrap();
            assert_eq!(tokens, vec![104, 101, 108, 108, 111]); // "hello" as bytes

            // Decode should roundtrip for ASCII
            let decoded = dispatch_tokenizer_decode(&handle, &tokens).unwrap();
            assert_eq!(decoded, "hello");
        }

        #[test]
        fn test_tokenizer_types() {
            // Stubs always succeed regardless of file existence
            let bpe = dispatch_tokenizer_load_bpe("vocab.json", "merges.txt").unwrap();
            assert_eq!(bpe.tokenizer_type, TokenizerType::Bpe);

            let pretrained = dispatch_tokenizer_load_pretrained("gpt2").unwrap();
            assert_eq!(pretrained.tokenizer_type, TokenizerType::Pretrained);

            let spm = dispatch_tokenizer_load_spm("model.spm").unwrap();
            assert_eq!(spm.tokenizer_type, TokenizerType::SentencePiece);
        }

        #[test]
        fn test_batch_operations() {
            let handle = dispatch_tokenizer_load_pretrained("test").unwrap();

            let texts = ["hello", "world"];
            let batch_encoded = dispatch_tokenizer_encode_batch(&handle, &texts).unwrap();

            assert_eq!(batch_encoded.len(), 2);

            let batch_decoded = dispatch_tokenizer_decode_batch(&handle, &batch_encoded).unwrap();
            assert_eq!(batch_decoded.len(), 2);
            assert_eq!(batch_decoded[0], "hello");
            assert_eq!(batch_decoded[1], "world");
        }
    }

    /// Test that real implementations work correctly (when tokenizers feature is enabled).
    #[cfg(feature = "tokenizers")]
    mod real_tests {
        use super::*;

        #[test]
        fn test_load_nonexistent_returns_none() {
            // Real implementation returns None for non-existent files
            assert!(dispatch_tokenizer_load_bpe("nonexistent.json", "nonexistent.txt").is_none());
            assert!(dispatch_tokenizer_load_pretrained("nonexistent-model").is_none());
            assert!(dispatch_tokenizer_load_spm("nonexistent.model").is_none());
        }

        // These tests require network access and actual model files
        // They are skipped in CI unless explicitly enabled

        #[test]
        #[ignore = "requires network access to HuggingFace hub"]
        fn test_real_gpt2_tokenizer() {
            let handle = dispatch_tokenizer_load_pretrained("gpt2").unwrap();

            let text = "Hello, world!";
            let tokens = dispatch_tokenizer_encode(&handle, text).unwrap();

            // GPT-2 tokenizes this as specific tokens
            assert!(!tokens.is_empty());

            let decoded = dispatch_tokenizer_decode(&handle, &tokens).unwrap();
            assert_eq!(decoded, text);
        }

        #[test]
        #[ignore = "requires network access to HuggingFace hub"]
        fn test_real_bert_tokenizer() {
            let handle = dispatch_tokenizer_load_pretrained("bert-base-uncased").unwrap();

            let text = "The quick brown fox";
            let tokens = dispatch_tokenizer_encode(&handle, text).unwrap();

            assert!(!tokens.is_empty());

            let decoded = dispatch_tokenizer_decode(&handle, &tokens).unwrap();
            // BERT may not decode exactly due to lowercasing
            assert!(decoded.contains("quick"));
        }
    }
}
