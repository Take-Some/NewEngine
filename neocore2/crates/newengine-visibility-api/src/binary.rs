use crate::{
    VisibilityObservationV1, VisibilityQueryBatchV1, VisibilityQueryCandidateV1,
    VisibilityResultBatchV1, VisibilitySphereV1, VisibilitySubjectResultV1, VisibilityVec3V1,
    VisibilityViewV1,
};

const QUERY_MAGIC: &[u8; 8] = b"NEVQ\x01\0\0\0";
const RESULT_MAGIC: &[u8; 8] = b"NEVR\x01\0\0\0";
const MAX_BINARY_CANDIDATES: usize = 1 << 20;
const MAX_BINARY_RESULTS: usize = 1 << 20;
const MAX_DIAGNOSTICS: usize = 256;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

pub fn encode_visibility_query_batch_bin(batch: &VisibilityQueryBatchV1) -> Result<Vec<u8>, String> {
    if batch.candidates.len() > MAX_BINARY_CANDIDATES {
        return Err(format!(
            "visibility binary query candidate count {} exceeds {}",
            batch.candidates.len(),
            MAX_BINARY_CANDIDATES
        ));
    }
    let max_results = u32::try_from(batch.max_results)
        .map_err(|_| "visibility max_results exceeds u32".to_owned())?;
    let count = u32::try_from(batch.candidates.len())
        .map_err(|_| "visibility candidate count exceeds u32".to_owned())?;
    let mut out = Vec::with_capacity(64 + batch.candidates.len() * 36);
    out.extend_from_slice(QUERY_MAGIC);
    put_u64(&mut out, batch.frame);
    put_vec3(&mut out, batch.view.position);
    put_vec3(&mut out, batch.view.forward);
    put_f32(&mut out, batch.view.max_distance);
    put_f32(&mut out, batch.view.coarse_cone_dot);
    put_u32(&mut out, max_results);
    put_u32(&mut out, count);
    for candidate in &batch.candidates {
        put_u64(&mut out, candidate.subject_id);
        put_vec3(&mut out, candidate.bounds.center);
        put_f32(&mut out, candidate.bounds.radius);
        put_i32(&mut out, candidate.priority);
    }
    Ok(out)
}

pub fn decode_visibility_query_batch_bin(bytes: &[u8]) -> Result<VisibilityQueryBatchV1, String> {
    let mut r = Reader::new(bytes);
    r.magic(QUERY_MAGIC)?;
    let frame = r.u64()?;
    let position = r.vec3()?;
    let forward = r.vec3()?;
    let max_distance = r.f32()?;
    let coarse_cone_dot = r.f32()?;
    let max_results = r.u32()? as usize;
    let count = r.u32()? as usize;
    if count > MAX_BINARY_CANDIDATES {
        return Err(format!(
            "visibility binary query candidate count {count} exceeds {MAX_BINARY_CANDIDATES}"
        ));
    }
    let mut candidates = Vec::with_capacity(count);
    for _ in 0..count {
        candidates.push(VisibilityQueryCandidateV1 {
            subject_id: r.u64()?,
            bounds: VisibilitySphereV1 {
                center: r.vec3()?,
                radius: r.f32()?,
            },
            priority: r.i32()?,
        });
    }
    r.finish()?;
    Ok(VisibilityQueryBatchV1 {
        frame,
        view: VisibilityViewV1 {
            position,
            forward,
            max_distance,
            coarse_cone_dot,
        },
        candidates,
        max_results,
    })
}

pub fn encode_visibility_result_batch_bin(batch: &VisibilityResultBatchV1) -> Result<Vec<u8>, String> {
    if batch.results.len() > MAX_BINARY_RESULTS {
        return Err(format!(
            "visibility binary result count {} exceeds {}",
            batch.results.len(),
            MAX_BINARY_RESULTS
        ));
    }
    if batch.diagnostics.len() > MAX_DIAGNOSTICS {
        return Err(format!(
            "visibility diagnostics count {} exceeds {}",
            batch.diagnostics.len(),
            MAX_DIAGNOSTICS
        ));
    }
    let mut out = Vec::with_capacity(32 + batch.results.len() * 24);
    out.extend_from_slice(RESULT_MAGIC);
    put_u64(&mut out, batch.provider_frame);
    put_u32(&mut out, batch.results.len() as u32);
    for result in &batch.results {
        put_u64(&mut out, result.subject_id);
        put_u8(
            &mut out,
            match result.observation {
                VisibilityObservationV1::Unknown => 0,
                VisibilityObservationV1::Visible => 1,
                VisibilityObservationV1::Occluded => 2,
            },
        );
        out.extend_from_slice(&[0, 0, 0]);
        put_f32(&mut out, result.confidence);
        put_u64(&mut out, result.produced_frame);
    }
    put_u32(&mut out, batch.diagnostics.len() as u32);
    for diagnostic in &batch.diagnostics {
        let bytes = diagnostic.as_bytes();
        if bytes.len() > MAX_DIAGNOSTIC_BYTES {
            return Err(format!(
                "visibility diagnostic is {} bytes; max is {}",
                bytes.len(),
                MAX_DIAGNOSTIC_BYTES
            ));
        }
        put_u32(&mut out, bytes.len() as u32);
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

pub fn decode_visibility_result_batch_bin(bytes: &[u8]) -> Result<VisibilityResultBatchV1, String> {
    let mut r = Reader::new(bytes);
    r.magic(RESULT_MAGIC)?;
    let provider_frame = r.u64()?;
    let count = r.u32()? as usize;
    if count > MAX_BINARY_RESULTS {
        return Err(format!(
            "visibility binary result count {count} exceeds {MAX_BINARY_RESULTS}"
        ));
    }
    let mut results = Vec::with_capacity(count);
    for _ in 0..count {
        let subject_id = r.u64()?;
        let observation = match r.u8()? {
            0 => VisibilityObservationV1::Unknown,
            1 => VisibilityObservationV1::Visible,
            2 => VisibilityObservationV1::Occluded,
            other => return Err(format!("invalid visibility observation tag {other}")),
        };
        r.skip(3)?;
        results.push(VisibilitySubjectResultV1 {
            subject_id,
            observation,
            confidence: r.f32()?,
            produced_frame: r.u64()?,
        });
    }
    let diagnostics_count = r.u32()? as usize;
    if diagnostics_count > MAX_DIAGNOSTICS {
        return Err(format!(
            "visibility diagnostics count {diagnostics_count} exceeds {MAX_DIAGNOSTICS}"
        ));
    }
    let mut diagnostics = Vec::with_capacity(diagnostics_count);
    for _ in 0..diagnostics_count {
        let len = r.u32()? as usize;
        if len > MAX_DIAGNOSTIC_BYTES {
            return Err(format!(
                "visibility diagnostic length {len} exceeds {MAX_DIAGNOSTIC_BYTES}"
            ));
        }
        diagnostics.push(
            std::str::from_utf8(r.bytes(len)?)
                .map_err(|error| format!("visibility diagnostic is not UTF-8: {error}"))?
                .to_owned(),
        );
    }
    r.finish()?;
    Ok(VisibilityResultBatchV1 {
        provider_frame,
        results,
        diagnostics,
    })
}

#[inline]
fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}
#[inline]
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
#[inline]
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
#[inline]
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
#[inline]
fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_bits().to_le_bytes());
}
#[inline]
fn put_vec3(out: &mut Vec<u8>, value: VisibilityVec3V1) {
    put_f32(out, value.x);
    put_f32(out, value.y);
    put_f32(out, value.z);
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    #[inline]
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn magic(&mut self, expected: &[u8]) -> Result<(), String> {
        if self.bytes(expected.len())? != expected {
            return Err("invalid visibility binary magic/version".to_owned());
        }
        Ok(())
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or_else(|| "visibility binary cursor overflow".to_owned())?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| "truncated visibility binary payload".to_owned())?;
        self.cursor = end;
        Ok(value)
    }

    fn skip(&mut self, len: usize) -> Result<(), String> {
        let _ = self.bytes(len)?;
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let mut raw = [0_u8; 4];
        raw.copy_from_slice(self.bytes(4)?);
        Ok(u32::from_le_bytes(raw))
    }

    fn i32(&mut self) -> Result<i32, String> {
        let mut raw = [0_u8; 4];
        raw.copy_from_slice(self.bytes(4)?);
        Ok(i32::from_le_bytes(raw))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let mut raw = [0_u8; 8];
        raw.copy_from_slice(self.bytes(8)?);
        Ok(u64::from_le_bytes(raw))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn vec3(&mut self) -> Result<VisibilityVec3V1, String> {
        Ok(VisibilityVec3V1 {
            x: self.f32()?,
            y: self.f32()?,
            z: self.f32()?,
        })
    }

    fn finish(self) -> Result<(), String> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(format!(
                "visibility binary payload has {} trailing bytes",
                self.bytes.len() - self.cursor
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_binary_roundtrips() {
        let batch = VisibilityQueryBatchV1 {
            frame: 77,
            view: VisibilityViewV1 {
                position: VisibilityVec3V1 { x: 1.0, y: 2.0, z: 3.0 },
                forward: VisibilityVec3V1 { x: 0.0, y: 0.0, z: -1.0 },
                max_distance: 1200.0,
                coarse_cone_dot: -0.2,
            },
            candidates: vec![VisibilityQueryCandidateV1 {
                subject_id: 0x1234_5678_9abc_def0,
                bounds: VisibilitySphereV1 {
                    center: VisibilityVec3V1 { x: 4.0, y: 5.0, z: 6.0 },
                    radius: 7.0,
                },
                priority: 91,
            }],
            max_results: 2048,
        };
        let decoded = decode_visibility_query_batch_bin(&encode_visibility_query_batch_bin(&batch).unwrap()).unwrap();
        assert_eq!(decoded, batch);
    }

    #[test]
    fn result_binary_roundtrips() {
        let batch = VisibilityResultBatchV1 {
            provider_frame: 81,
            results: vec![VisibilitySubjectResultV1 {
                subject_id: 42,
                observation: VisibilityObservationV1::Occluded,
                confidence: 0.97,
                produced_frame: 79,
            }],
            diagnostics: vec!["gpu_latency_frames=2".to_owned()],
        };
        let decoded = decode_visibility_result_batch_bin(&encode_visibility_result_batch_bin(&batch).unwrap()).unwrap();
        assert_eq!(decoded, batch);
    }
}
