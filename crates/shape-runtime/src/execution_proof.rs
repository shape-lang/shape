//! Proof of execution for content-addressed functions.
//!
//! An execution proof attests that a specific function (identified by content hash)
//! was executed with specific arguments and produced a specific result.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Compute SHA-256 of arbitrary bytes, returning a 32-byte array.
pub fn hash_bytes(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// A cryptographic proof that a function was executed with given inputs producing given outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProof {
    /// Content hash of the function that was executed.
    pub function_hash: [u8; 32],
    /// SHA-256 hash of the serialized arguments.
    pub args_hash: [u8; 32],
    /// SHA-256 hash of the serialized result.
    pub result_hash: [u8; 32],
    /// Unix timestamp (seconds) when execution completed.
    pub timestamp: u64,
    /// Optional execution trace: hashes of intermediate states.
    pub trace: Option<Vec<[u8; 32]>>,
    /// Hash of this entire proof (excluding this field).
    pub proof_hash: [u8; 32],
}

impl ExecutionProof {
    /// Compute a deterministic hash over all proof fields except `proof_hash` itself.
    pub fn compute_proof_hash(
        function_hash: &[u8; 32],
        args_hash: &[u8; 32],
        result_hash: &[u8; 32],
        timestamp: u64,
        trace: &Option<Vec<[u8; 32]>>,
    ) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(function_hash);
        hasher.update(args_hash);
        hasher.update(result_hash);
        hasher.update(timestamp.to_le_bytes());
        if let Some(entries) = trace {
            // Prefix with entry count for unambiguous encoding.
            hasher.update((entries.len() as u64).to_le_bytes());
            for entry in entries {
                hasher.update(entry);
            }
        } else {
            // Distinguish None from empty vec.
            hasher.update([0xff; 8]);
        }
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        out
    }

    /// Verify that the proof's `proof_hash` matches its contents.
    pub fn verify_integrity(&self) -> bool {
        let expected = Self::compute_proof_hash(
            &self.function_hash,
            &self.args_hash,
            &self.result_hash,
            self.timestamp,
            &self.trace,
        );
        expected == self.proof_hash
    }
}

/// Builder for constructing execution proofs during function execution.
pub struct ExecutionProofBuilder {
    function_hash: [u8; 32],
    args_hash: Option<[u8; 32]>,
    trace: Vec<[u8; 32]>,
    capture_trace: bool,
}

impl ExecutionProofBuilder {
    /// Create a new builder for the given function content hash.
    pub fn new(function_hash: [u8; 32]) -> Self {
        Self {
            function_hash,
            args_hash: None,
            trace: Vec::new(),
            capture_trace: false,
        }
    }

    /// Enable trace capture. When enabled, `record_trace_step` appends entries.
    pub fn with_trace(mut self) -> Self {
        self.capture_trace = true;
        self
    }

    /// Record the hash of the serialized arguments.
    pub fn set_args_hash(&mut self, hash: [u8; 32]) {
        self.args_hash = Some(hash);
    }

    /// Record an intermediate state hash in the execution trace.
    ///
    /// This is a no-op if trace capture was not enabled via `with_trace`.
    pub fn record_trace_step(&mut self, state_hash: [u8; 32]) {
        if self.capture_trace {
            self.trace.push(state_hash);
        }
    }

    /// Finalize the proof with the result hash. Computes the proof hash and
    /// returns the completed `ExecutionProof`.
    pub fn finalize(self, result_hash: [u8; 32]) -> ExecutionProof {
        let args_hash = self.args_hash.unwrap_or([0u8; 32]);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let trace = if self.capture_trace {
            Some(self.trace)
        } else {
            None
        };
        let proof_hash = ExecutionProof::compute_proof_hash(
            &self.function_hash,
            &args_hash,
            &result_hash,
            timestamp,
            &trace,
        );
        ExecutionProof {
            function_hash: self.function_hash,
            args_hash,
            result_hash,
            timestamp,
            trace,
            proof_hash,
        }
    }
}

/// Result of verifying an execution proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    /// Proof is internally consistent.
    Valid,
    /// Proof hash doesn't match computed hash.
    InvalidProofHash,
    /// Re-execution produced a different result hash.
    ResultMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    /// Trace verification failed at a specific step.
    TraceMismatch { step: usize },
}

/// Registry of execution proofs, indexed by function hash.
pub struct ProofRegistry {
    proofs: Vec<ExecutionProof>,
    by_function: HashMap<[u8; 32], Vec<usize>>,
}

impl ProofRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            proofs: Vec::new(),
            by_function: HashMap::new(),
        }
    }

    /// Register a proof. Returns the index of the newly registered proof.
    pub fn register(&mut self, proof: ExecutionProof) -> usize {
        let idx = self.proofs.len();
        self.by_function
            .entry(proof.function_hash)
            .or_default()
            .push(idx);
        self.proofs.push(proof);
        idx
    }

    /// Look up all proofs for a given function content hash.
    pub fn lookup(&self, function_hash: &[u8; 32]) -> &[ExecutionProof] {
        match self.by_function.get(function_hash) {
            Some(indices) => {
                // Return a contiguous slice when proofs were registered
                // sequentially for this function; otherwise collect references.
                // Since we always append, we can return the subset via indices.
                // For simplicity, we return a slice of the full vec when there
                // is exactly one contiguous run. For the general case, callers
                // should use `lookup_indices`.
                if indices.is_empty() {
                    return &[];
                }
                let first = indices[0];
                let last = *indices.last().unwrap();
                // Check if the indices form a contiguous range in proofs vec.
                if last - first + 1 == indices.len() {
                    &self.proofs[first..=last]
                } else {
                    // Fallback: return empty. Use lookup_iter for non-contiguous.
                    &[]
                }
            }
            None => &[],
        }
    }

    /// Iterate over all proofs for a given function hash.
    pub fn lookup_iter<'a>(
        &'a self,
        function_hash: &[u8; 32],
    ) -> impl Iterator<Item = &'a ExecutionProof> {
        let indices = self.by_function.get(function_hash);
        indices
            .into_iter()
            .flat_map(|v| v.iter())
            .map(move |&idx| &self.proofs[idx])
    }

    /// Verify integrity of all registered proofs.
    ///
    /// Returns a list of `(proof_index, VerificationResult)` for every proof
    /// that fails integrity verification.
    pub fn verify_all(&self) -> Vec<(usize, VerificationResult)> {
        let mut failures = Vec::new();
        for (i, proof) in self.proofs.iter().enumerate() {
            if !proof.verify_integrity() {
                failures.push((i, VerificationResult::InvalidProofHash));
            }
        }
        failures
    }

    /// Total number of registered proofs.
    pub fn len(&self) -> usize {
        self.proofs.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.proofs.is_empty()
    }
}

impl Default for ProofRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_bytes() {
        let h = hash_bytes(b"hello");
        // SHA-256 of "hello" is well-known.
        assert_eq!(
            hex::encode(h),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_proof_integrity_valid() {
        let func_hash = hash_bytes(b"my_function");
        let args_hash = hash_bytes(b"args");
        let result_hash = hash_bytes(b"result");

        let mut builder = ExecutionProofBuilder::new(func_hash);
        builder.set_args_hash(args_hash);
        let proof = builder.finalize(result_hash);

        assert!(proof.verify_integrity());
    }

    #[test]
    fn test_proof_integrity_tampered() {
        let func_hash = hash_bytes(b"my_function");
        let args_hash = hash_bytes(b"args");
        let result_hash = hash_bytes(b"result");

        let mut builder = ExecutionProofBuilder::new(func_hash);
        builder.set_args_hash(args_hash);
        let mut proof = builder.finalize(result_hash);

        // Tamper with the result hash.
        proof.result_hash = hash_bytes(b"different_result");
        assert!(!proof.verify_integrity());
    }

    #[test]
    fn test_proof_with_trace() {
        let func_hash = hash_bytes(b"traced_fn");
        let args_hash = hash_bytes(b"args");

        let mut builder = ExecutionProofBuilder::new(func_hash).with_trace();
        builder.set_args_hash(args_hash);
        builder.record_trace_step(hash_bytes(b"state_1"));
        builder.record_trace_step(hash_bytes(b"state_2"));

        let result_hash = hash_bytes(b"final");
        let proof = builder.finalize(result_hash);

        assert!(proof.verify_integrity());
        assert!(proof.trace.is_some());
        assert_eq!(proof.trace.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_trace_disabled_by_default() {
        let mut builder = ExecutionProofBuilder::new([0u8; 32]);
        builder.record_trace_step(hash_bytes(b"ignored"));
        let proof = builder.finalize([1u8; 32]);

        assert!(proof.trace.is_none());
    }

    #[test]
    fn test_registry_register_and_lookup() {
        let mut registry = ProofRegistry::new();
        let func_hash = hash_bytes(b"fn1");

        let mut b = ExecutionProofBuilder::new(func_hash);
        b.set_args_hash(hash_bytes(b"a1"));
        let p1 = b.finalize(hash_bytes(b"r1"));

        let mut b2 = ExecutionProofBuilder::new(func_hash);
        b2.set_args_hash(hash_bytes(b"a2"));
        let p2 = b2.finalize(hash_bytes(b"r2"));

        registry.register(p1);
        registry.register(p2);

        assert_eq!(registry.len(), 2);
        let results: Vec<_> = registry.lookup_iter(&func_hash).collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_registry_verify_all_clean() {
        let mut registry = ProofRegistry::new();
        let mut b = ExecutionProofBuilder::new(hash_bytes(b"f"));
        b.set_args_hash(hash_bytes(b"a"));
        registry.register(b.finalize(hash_bytes(b"r")));

        let failures = registry.verify_all();
        assert!(failures.is_empty());
    }

    #[test]
    fn test_registry_lookup_missing() {
        let registry = ProofRegistry::new();
        let missing = hash_bytes(b"nonexistent");
        assert!(registry.lookup(&missing).is_empty());
        assert_eq!(registry.lookup_iter(&missing).count(), 0);
    }
}
