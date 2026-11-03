//! A single JSON byte budget shared with report consumers. Count actual escaped
//! bytes before allocating output; preserve complete records and exact identity.
use serde::Serialize;
use std::io::{self, Write};
use verum_common::{
    List,
    runtime_diagnostics::{MAX_RUNTIME_REPORT_BYTES, RuntimeReport},
};

#[derive(Default)]
struct Size(usize);

impl Write for Size {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("Runtime report size overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_size(value: &impl Serialize) -> Result<usize, String> {
    let mut size = Size::default();
    serde_json::to_writer(&mut size, value).map_err(|error| error.to_string())?;
    Ok(size.0)
}

struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other(
                "Runtime report exceeds its serialized byte budget",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode_bounded(report: RuntimeReport) -> Result<Vec<u8>, String> {
    encode_with_limit(report, MAX_RUNTIME_REPORT_BYTES as usize)
}

fn encode_with_limit(mut report: RuntimeReport, limit: usize) -> Result<Vec<u8>, String> {
    report.validate_binding(&report.run_id, &report.source_sha256)?;
    let mut loss = report
        .export_loss
        .ok_or("Bounded export requires runtime report v2")?;
    let sizes = |rows: &[_]| rows.iter().map(json_size).collect::<Result<List<_>, _>>();
    let mut cbgr = sizes(report.cbgr.as_ref().map_or(&[], |r| r.events.as_slice()))?;
    let mut methods = report
        .method_dispatch
        .as_ref()
        .map(|r| r.rows.iter().map(json_size).collect::<Result<List<_>, _>>())
        .transpose()?
        .unwrap_or_default();
    let tasks = report
        .async_tasks
        .as_ref()
        .map(|r| r.rows.iter().map(json_size).collect::<Result<List<_>, _>>())
        .transpose()?
        .unwrap_or_default();
    let mut task_start = 0;
    let total = |omitted: usize, retained: usize| {
        omitted
            .checked_add(retained)
            .ok_or("Runtime report export count overflow")
    };
    // Reserve the largest possible accounting fields, including decimal growth.
    // All other metadata stays byte-for-byte intact throughout export.
    report.export_loss = Some(verum_common::runtime_diagnostics::RuntimeExportLoss {
        cbgr_events: total(loss.cbgr_events, cbgr.len())?,
        method_sites: total(loss.method_sites, methods.len())?,
        task_rows: total(loss.task_rows, tasks.len())?,
    });
    let mut bound = json_size(&report)?;
    while bound > limit {
        // Keep the first CBGR/site records and the most recent tasks. Among
        // these boundaries remove the largest record, never a partial error.
        let candidates = [
            cbgr.last().copied(),
            methods.last().copied(),
            tasks.get(task_start).copied(),
        ];
        let (section, bytes) = candidates
            .into_iter()
            .enumerate()
            .filter_map(|(section, bytes)| bytes.map(|bytes| (section, bytes)))
            .max_by_key(|(_, bytes)| *bytes)
            .ok_or("Runtime report identity exceeds its serialized byte budget")?;
        let had_comma = match section {
            0 => {
                let comma = cbgr.len() > 1;
                cbgr.pop();
                report.cbgr.as_mut().expect("CBGR row exists").events.pop();
                loss.cbgr_events += 1;
                comma
            }
            1 => {
                let comma = methods.len() > 1;
                methods.pop();
                report
                    .method_dispatch
                    .as_mut()
                    .expect("method row exists")
                    .rows
                    .pop();
                loss.method_sites += 1;
                comma
            }
            _ => {
                let comma = tasks.len() - task_start > 1;
                task_start += 1;
                loss.task_rows += 1;
                comma
            }
        };
        bound -= bytes + usize::from(had_comma);
    }
    if let Some(tasks) = &mut report.async_tasks {
        // Remove the omitted prefix once: linear work even when every task is
        // omitted. The error strings of retained tasks are never edited.
        drop(tasks.rows.drain(..task_start));
    }
    report.export_loss = Some(loss);
    report.validate_binding(&report.run_id, &report.source_sha256)?;
    let mut output = BoundedOutput {
        bytes: Vec::with_capacity(bound),
        limit,
    };
    serde_json::to_writer(&mut output, &report).map_err(|error| error.to_string())?;
    Ok(output.bytes)
}

#[cfg(test)]
#[path = "../../tests/unit/runtime_report_transport.rs"]
mod tests;
