//! What the guest reaches on the host: the processes SE mode runs, the
//! files of the readfile and writefile pseudo-operations, the console and
//! the logs.

use crate::soc::terminal::SharedTerminal;
use crate::system::host_files::HostFiles;
use crate::system::process::Processes;
use crate::uarch::pipeline::trace::PipelineTrace;

/// What the guest reaches on the host.
#[derive(Debug)]
pub struct HostSide {
    /// The processes SE mode is running and the harts their threads hold;
    /// empty otherwise.
    pub processes: Processes,
    /// The script file and output directory the guest reaches with the
    /// readfile and writefile pseudo-operations.
    pub host_files: HostFiles,
    /// The console every device and proxy prints to and reads from.
    pub terminal: SharedTerminal,
    /// Optional buffered writer for the commit log (enabled by the
    /// `commit-log` feature).
    #[cfg(feature = "commit-log")]
    pub commit_log: Option<std::io::BufWriter<std::fs::File>>,
    /// Where the pipeline reports each instruction's progress, when a
    /// trace is open.
    pub pipeline_trace: Option<Box<dyn PipelineTrace>>,
}

impl HostSide {
    /// No processes, files or logs yet, and the console on `terminal`.
    #[must_use]
    pub fn new(terminal: SharedTerminal) -> Self {
        Self {
            processes: Processes::default(),
            host_files: HostFiles::default(),
            terminal,
            #[cfg(feature = "commit-log")]
            commit_log: None,
            pipeline_trace: None,
        }
    }
}
