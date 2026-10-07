use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteGpuOverview {
    pub available: bool,
    pub driver_version: String,
    pub cuda_version: String,
    pub gpus: Vec<RemoteGpu>,
    pub processes: Vec<RemoteGpuProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteGpu {
    pub index: u32,
    pub uuid: String,
    pub name: String,
    pub temperature_c: Option<f64>,
    pub utilization_gpu_percent: Option<f64>,
    pub utilization_memory_percent: Option<f64>,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub memory_free_mb: u64,
    pub power_draw_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub fan_speed_percent: Option<f64>,
    pub pstate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteGpuProcess {
    pub gpu_uuid: String,
    pub gpu_index: Option<u32>,
    pub pid: u32,
    pub process_name: String,
    pub used_memory_mb: u64,
}

enum GpuParseSection {
    None,
    Gpus,
    Processes,
}

pub fn parse_gpu_overview_output(output: &str) -> RemoteGpuOverview {
    let mut overview = RemoteGpuOverview::default();
    let mut section = GpuParseSection::None;
    let mut process_rows = Vec::new();

    for line in output.lines() {
        match line {
            "GPU_CSV_BEGIN" => {
                section = GpuParseSection::Gpus;
                continue;
            }
            "GPU_CSV_END" => {
                section = GpuParseSection::None;
                continue;
            }
            "GPU_PROCESS_CSV_BEGIN" => {
                section = GpuParseSection::Processes;
                continue;
            }
            "GPU_PROCESS_CSV_END" => {
                section = GpuParseSection::None;
                continue;
            }
            _ => {}
        }

        let cols: Vec<&str> = line.split('\t').collect();
        if cols.first() == Some(&"GPU_AVAILABLE") && cols.len() >= 2 {
            overview.available = cols[1] == "1";
            continue;
        }
        if cols.first() == Some(&"GPU_CUDA_VERSION") && cols.len() >= 2 {
            overview.cuda_version = cols[1].trim().to_string();
            continue;
        }

        match section {
            GpuParseSection::Gpus => {
                if let Some(gpu) = parse_gpu_csv_line(line) {
                    if overview.driver_version.is_empty() {
                        overview.driver_version = gpu.1;
                    }
                    overview.gpus.push(gpu.0);
                }
            }
            GpuParseSection::Processes => {
                if !line.trim().is_empty() {
                    process_rows.push(line.to_string());
                }
            }
            GpuParseSection::None => {}
        }
    }

    let gpu_indexes: HashMap<String, u32> = overview
        .gpus
        .iter()
        .map(|gpu| (gpu.uuid.clone(), gpu.index))
        .collect();
    overview.processes = process_rows
        .iter()
        .filter_map(|line| parse_gpu_process_csv_line(line, &gpu_indexes))
        .collect();

    overview
}

fn parse_gpu_csv_line(line: &str) -> Option<(RemoteGpu, String)> {
    let cols = parse_csv_line(line);
    if cols.len() < 14 {
        return None;
    }

    let gpu = RemoteGpu {
        index: parse_u32(&cols[0]).unwrap_or(0),
        uuid: cols[1].trim().to_string(),
        name: cols[2].trim().to_string(),
        temperature_c: parse_optional_f64(&cols[4]),
        utilization_gpu_percent: parse_optional_f64(&cols[5]),
        utilization_memory_percent: parse_optional_f64(&cols[6]),
        memory_total_mb: parse_u64(&cols[7]).unwrap_or(0),
        memory_used_mb: parse_u64(&cols[8]).unwrap_or(0),
        memory_free_mb: parse_u64(&cols[9]).unwrap_or(0),
        power_draw_w: parse_optional_f64(&cols[10]),
        power_limit_w: parse_optional_f64(&cols[11]),
        fan_speed_percent: parse_optional_f64(&cols[12]),
        pstate: clean_text(&cols[13]),
    };

    Some((gpu, clean_text(&cols[3])))
}

fn parse_gpu_process_csv_line(
    line: &str,
    gpu_indexes: &HashMap<String, u32>,
) -> Option<RemoteGpuProcess> {
    let cols = parse_csv_line(line);
    if cols.len() < 4 {
        return None;
    }

    let gpu_uuid = clean_text(&cols[0]);
    if gpu_uuid.is_empty() || is_missing(&gpu_uuid) {
        return None;
    }

    Some(RemoteGpuProcess {
        gpu_index: gpu_indexes.get(&gpu_uuid).copied(),
        gpu_uuid,
        pid: parse_u32(&cols[1]).unwrap_or(0),
        used_memory_mb: parse_u64(&cols[2]).unwrap_or(0),
        process_name: cols[3..].join(",").trim().to_string(),
    })
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '"' if in_quotes && chars.get(index + 1) == Some(&'"') => {
                current.push('"');
                index += 1;
            }
            '"' => {
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                values.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
        index += 1;
    }

    values.push(current.trim().to_string());
    values
}

fn clean_text(value: &str) -> String {
    value.trim().to_string()
}

fn is_missing(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "" | "n/a" | "na" | "[n/a]" | "[not supported]" | "not supported"
    )
}

fn parse_optional_f64(value: &str) -> Option<f64> {
    if is_missing(value) {
        return None;
    }
    numeric_prefix(value).and_then(|value| value.parse().ok())
}

fn parse_u64(value: &str) -> Option<u64> {
    if is_missing(value) {
        return None;
    }
    numeric_prefix(value).and_then(|value| value.parse::<f64>().ok().map(|number| number as u64))
}

fn parse_u32(value: &str) -> Option<u32> {
    parse_u64(value).and_then(|value| u32::try_from(value).ok())
}

fn numeric_prefix(value: &str) -> Option<&str> {
    let trimmed = value.trim().trim_end_matches('%').trim();
    let end = trimmed
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit() || *ch == '.' || *ch == '-')
        .map(|(index, ch)| index + ch.len_utf8())
        .last()
        .unwrap_or(0);

    (end > 0).then_some(&trimmed[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unavailable_gpu_output() {
        let overview = parse_gpu_overview_output("GPU_AVAILABLE\t0\n");

        assert!(!overview.available);
        assert!(overview.gpus.is_empty());
        assert!(overview.processes.is_empty());
    }

    #[test]
    fn parses_single_gpu_with_metrics() {
        let raw = concat!(
            "GPU_AVAILABLE\t1\n",
            "GPU_CUDA_VERSION\t12.4\n",
            "GPU_CSV_BEGIN\n",
            "0, GPU-abc, NVIDIA RTX 4090, 550.54.14, 43, 77, 31, 24564, 12345, 12219, 180.5, 450.0, 46, P2\n",
            "GPU_CSV_END\n",
            "GPU_PROCESS_CSV_BEGIN\n",
            "GPU_PROCESS_CSV_END\n",
        );

        let overview = parse_gpu_overview_output(raw);

        assert!(overview.available);
        assert_eq!(overview.driver_version, "550.54.14");
        assert_eq!(overview.cuda_version, "12.4");
        assert_eq!(overview.gpus[0].name, "NVIDIA RTX 4090");
        assert_eq!(overview.gpus[0].utilization_gpu_percent, Some(77.0));
        assert_eq!(overview.gpus[0].memory_used_mb, 12345);
        assert_eq!(overview.gpus[0].power_draw_w, Some(180.5));
    }

    #[test]
    fn parses_multiple_gpus_and_missing_optional_metrics() {
        let raw = concat!(
            "GPU_AVAILABLE\t1\n",
            "GPU_CUDA_VERSION\t12.2\n",
            "GPU_CSV_BEGIN\n",
            "0, GPU-a, NVIDIA A100, 535.1, N/A, 0, 0, 40536, 0, 40536, [Not Supported], 400, N/A, P0\n",
            "1, GPU-b, NVIDIA T4, 535.1, 55, 99, 80, 15360, 15000, 360, 67.2, 70.0, [Not Supported], P0\n",
            "GPU_CSV_END\n",
            "GPU_PROCESS_CSV_BEGIN\n",
            "GPU_PROCESS_CSV_END\n",
        );

        let overview = parse_gpu_overview_output(raw);

        assert_eq!(overview.gpus.len(), 2);
        assert_eq!(overview.gpus[0].temperature_c, None);
        assert_eq!(overview.gpus[0].power_draw_w, None);
        assert_eq!(overview.gpus[0].fan_speed_percent, None);
        assert_eq!(overview.gpus[1].fan_speed_percent, None);
    }

    #[test]
    fn maps_gpu_processes_to_indexes() {
        let raw = concat!(
            "GPU_AVAILABLE\t1\n",
            "GPU_CUDA_VERSION\t12.2\n",
            "GPU_CSV_BEGIN\n",
            "0, GPU-a, NVIDIA A100, 535.1, 40, 30, 20, 40536, 1000, 39536, 80, 400, 20, P0\n",
            "1, GPU-b, NVIDIA T4, 535.1, 44, 10, 9, 15360, 512, 14848, 35, 70, 30, P8\n",
            "GPU_CSV_END\n",
            "GPU_PROCESS_CSV_BEGIN\n",
            "GPU-b, 4242, 2048, python\n",
            "GPU_PROCESS_CSV_END\n",
        );

        let overview = parse_gpu_overview_output(raw);

        assert_eq!(overview.processes.len(), 1);
        assert_eq!(overview.processes[0].gpu_index, Some(1));
        assert_eq!(overview.processes[0].pid, 4242);
        assert_eq!(overview.processes[0].used_memory_mb, 2048);
    }

    #[test]
    fn keeps_commas_inside_quoted_process_names() {
        let raw = concat!(
            "GPU_AVAILABLE\t1\n",
            "GPU_CUDA_VERSION\t12.2\n",
            "GPU_CSV_BEGIN\n",
            "0, GPU-a, NVIDIA A100, 535.1, 40, 30, 20, 40536, 1000, 39536, 80, 400, 20, P0\n",
            "GPU_CSV_END\n",
            "GPU_PROCESS_CSV_BEGIN\n",
            "GPU-a, 100, 512, \"python, worker\"\n",
            "GPU_PROCESS_CSV_END\n",
        );

        let overview = parse_gpu_overview_output(raw);

        assert_eq!(overview.processes[0].process_name, "python, worker");
    }
}
