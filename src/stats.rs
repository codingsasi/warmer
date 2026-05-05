use colored::*;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Default)]
pub struct Stats {
    transactions: usize,
    successful_transactions: usize,
    failed_transactions: usize,
    response_times: Vec<f64>,
    data_transferred: u64,
    start_time: Option<Instant>,
    end_time: Option<Instant>,
    status_codes: HashMap<u16, usize>,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            start_time: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn add_transaction(&mut self, response_time: f64, data_size: u64, status_code: u16) {
        self.transactions += 1;
        self.response_times.push(response_time);
        self.data_transferred += data_size;

        if status_code < 400 {
            self.successful_transactions += 1;
        } else {
            self.failed_transactions += 1;
        }

        *self.status_codes.entry(status_code).or_insert(0) += 1;
    }

    pub fn finish(&mut self) {
        self.end_time = Some(Instant::now());
    }

    fn elapsed_time(&self) -> f64 {
        if let (Some(start), Some(end)) = (self.start_time, self.end_time) {
            end.duration_since(start).as_secs_f64()
        } else if let Some(start) = self.start_time {
            start.elapsed().as_secs_f64()
        } else {
            0.0
        }
    }

    fn avg_response_time(&self) -> f64 {
        if self.response_times.is_empty() {
            0.0
        } else {
            self.response_times.iter().sum::<f64>() / self.response_times.len() as f64
        }
    }

    fn transaction_rate(&self) -> f64 {
        let elapsed = self.elapsed_time();
        if elapsed > 0.0 {
            self.transactions as f64 / elapsed
        } else {
            0.0
        }
    }

    fn throughput(&self) -> f64 {
        let elapsed = self.elapsed_time();
        if elapsed > 0.0 {
            self.data_transferred as f64 / elapsed / 1024.0 / 1024.0
        } else {
            0.0
        }
    }

    fn concurrency(&self) -> f64 {
        if self.response_times.is_empty() {
            0.0
        } else {
            self.avg_response_time() * self.transaction_rate() / 1000.0
        }
    }

    fn availability(&self) -> f64 {
        if self.transactions == 0 {
            0.0
        } else {
            (self.successful_transactions as f64 / self.transactions as f64) * 100.0
        }
    }
}

fn color_status_code(status_code: u16) -> ColoredString {
    match status_code {
        200..=299 => status_code.to_string().green(),
        300..=399 => status_code.to_string().yellow(),
        400..=499 => status_code.to_string().red(),
        500..=599 => status_code.to_string().red().bold(),
        _ => status_code.to_string().white(),
    }
}

fn format_response_time(ms: f64) -> String {
    format!("{:.2} secs", ms / 1000.0)
}

fn format_data_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} bytes", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn print_header(concurrent: usize, _url: &str) {
    println!("** WARMER 0.1.2");
    println!("** Preparing {} concurrent users for battle.", concurrent);
    println!("The server is now under load...");
}

pub fn print_transaction(
    status_code: u16,
    response_time: f64,
    data_size: u64,
    method: &str,
    path: &str,
    _verbose: bool,
    is_main_url: bool,
    http_version: &str,
) {
    let status_colored = color_status_code(status_code);
    let response_time_str = format_response_time(response_time);
    let data_size_str = format_data_size(data_size);

    if is_main_url {
        println!(
            "{} {}     {}: {} ==> {}  {}",
            http_version,
            status_colored.bold(),
            response_time_str.bold(),
            data_size_str.bold(),
            method.bold(),
            path.bold().bright_blue()
        );
    } else {
        println!(
            "{} {}     {}: {} ==> {}  {}",
            http_version, status_colored, response_time_str, data_size_str, method, path
        );
    }
}

pub fn print_statistics(stats: &Stats) {
    println!("\nLoad testing completed...");
    println!();
    println!("Transactions:\t\t{:8} hits", stats.transactions);
    println!("Availability:\t\t{:8.2} %", stats.availability());
    println!("Elapsed time:\t\t{:8.2} secs", stats.elapsed_time());
    println!(
        "Data transferred:\t{:8.2} MB",
        stats.data_transferred as f64 / (1024.0 * 1024.0)
    );
    println!("Response time:\t\t{:8.2} ms", stats.avg_response_time());
    println!(
        "Transaction rate:\t{:8.2} trans/sec",
        stats.transaction_rate()
    );
    println!("Throughput:\t\t{:8.2} MB/sec", stats.throughput());
    println!("Concurrency:\t\t{:8.2}", stats.concurrency());
    println!(
        "Successful transactions: {:8}",
        stats.successful_transactions
    );
    println!("Failed transactions:\t{:8}", stats.failed_transactions);

    if let Some(&max_time) = stats
        .response_times
        .iter()
        .max_by(|a, b| a.partial_cmp(b).unwrap())
    {
        println!("Longest transaction:\t{:8.2} ms", max_time);
    }

    if let Some(&min_time) = stats
        .response_times
        .iter()
        .min_by(|a, b| a.partial_cmp(b).unwrap())
    {
        println!("Shortest transaction:\t{:8.2} ms", min_time);
    }

    println!();
}
