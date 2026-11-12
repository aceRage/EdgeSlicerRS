//! The "Print host upload queue" (PrintHost.cpp `PrintHostJobQueue`,
//! PrintHostDialogs.cpp `PrintHostQueueDialog`): the files sent, one after
//! another on a thread of their own, each with its progress, its state and
//! what went wrong; one waiting or going can be called off.

use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::Ordering,
    mpsc::{Sender, channel},
};

use serde_json::{Value, json};

use crate::api::i18n::tr;

use super::{Failure, Host, Progress, Upload};

/// A job's state, in the queue's words (`PrintHostQueueDialog::set_state`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Queued,
    Uploading,
    Error,
    Canceling,
    Canceled,
    Completed,
}

impl State {
    pub fn said(self) -> &'static str {
        match self {
            State::Queued => "Queued",
            State::Uploading => "Uploading",
            State::Error => "Error",
            State::Canceling => "Canceling",
            State::Canceled => "Canceled",
            State::Completed => "Completed",
        }
    }

    /// Whether it is over.
    pub fn done(self) -> bool {
        matches!(self, State::Error | State::Canceled | State::Completed)
    }
}

struct Job {
    id: u32,
    name: String,
    /// The printer's address as shown: never with a user or password.
    host: String,
    printer: String,
    size: u64,
    start: bool,
    state: State,
    progress: Arc<Progress>,
    message: String,
}

/// A job to send, with the language of the call that queued it, in which
/// what goes wrong is said.
type Work = (
    u32,
    Box<dyn Host>,
    Upload,
    Arc<Progress>,
    Arc<crate::api::i18n::Catalog>,
);

#[derive(Default)]
pub struct Queue {
    jobs: Arc<Mutex<Vec<Job>>>,
    worker: Mutex<Option<Sender<Work>>>,
    next: Mutex<u32>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Queue {
    /// Adds a file to send and comes straight back with its number.
    pub fn add(&self, host: Box<dyn Host>, shown: &str, printer: &str, upload: Upload) -> u32 {
        let id = {
            let mut next = lock(&self.next);
            *next += 1;
            *next
        };
        let progress = Arc::new(Progress::default());
        progress
            .total
            .store(upload.data.len() as u64, Ordering::Relaxed);
        lock(&self.jobs).push(Job {
            id,
            name: upload.name.clone(),
            host: shown.to_owned(),
            printer: printer.to_owned(),
            size: upload.data.len() as u64,
            start: upload.start,
            state: State::Queued,
            progress: progress.clone(),
            message: String::new(),
        });
        let mut worker = lock(&self.worker);
        let sender = worker.get_or_insert_with(|| {
            let (sender, receiver) = channel::<Work>();
            let jobs = self.jobs.clone();
            std::thread::spawn(move || {
                for (id, host, upload, progress, language) in receiver {
                    crate::api::i18n::with(language, || {
                        run(&jobs, id, host.as_ref(), &upload, &progress);
                    });
                }
            });
            sender
        });
        let language = crate::api::i18n::current();
        let _ = sender.send((id, host, upload, progress, language));
        id
    }

    /// "Cancel selected": a job waiting is not sent; one going is stopped.
    pub fn cancel(&self, id: u32) -> Result<(), String> {
        let mut jobs = lock(&self.jobs);
        let job = jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("there is no upload {id}"))?;
        match job.state {
            State::Queued => {
                job.progress.cancel.store(true, Ordering::Relaxed);
                job.state = State::Canceled;
            }
            State::Uploading => {
                job.progress.cancel.store(true, Ordering::Relaxed);
                job.state = State::Canceling;
            }
            _ => {}
        }
        Ok(())
    }

    /// Takes the jobs that are over off the list.
    pub fn clear_done(&self) {
        lock(&self.jobs).retain(|job| !job.state.done());
    }

    /// Every job, oldest first: `{ id, name, host, printer, size, start,
    /// state, said, progress (0–100), stage, message }`.
    pub fn listing(&self) -> Value {
        let jobs = lock(&self.jobs);
        let list: Vec<Value> = jobs
            .iter()
            .map(|job| {
                let total = job.progress.total.load(Ordering::Relaxed).max(1);
                let sent = job.progress.sent.load(Ordering::Relaxed);
                let percent = match job.state {
                    State::Completed => 100,
                    State::Error | State::Canceled => 0,
                    _ => (sent * 100 / total).min(99),
                };
                json!({
                    "id": job.id,
                    "name": job.name,
                    "host": job.host,
                    "printer": job.printer,
                    "size": job.size,
                    "start": job.start,
                    "state": format!("{:?}", job.state).to_lowercase(),
                    "said": tr(job.state.said()),
                    "progress": percent,
                    "stage": lock(&job.progress.stage).clone(),
                    "message": job.message,
                })
            })
            .collect();
        json!({ "jobs": list, "busy": jobs.iter().any(|job| !job.state.done()) })
    }
}

fn run(jobs: &Mutex<Vec<Job>>, id: u32, host: &dyn Host, upload: &Upload, progress: &Progress) {
    {
        let mut jobs = lock(jobs);
        let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
            return;
        };
        if job.state != State::Queued {
            return;
        }
        job.state = State::Uploading;
    }
    let result = host.upload(upload, progress);
    let mut jobs = lock(jobs);
    let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
        return;
    };
    match result {
        Ok(()) => {
            job.state = State::Completed;
            job.message = if upload.start {
                "The print was started.".into()
            } else {
                String::new()
            };
        }
        Err(_) if progress.cancelled() => job.state = State::Canceled,
        Err(Failure::Cancelled) => job.state = State::Canceled,
        Err(Failure::Error(error)) => {
            job.state = State::Error;
            // PrintHostDialogs.cpp l.2568: "%1%\n%2%".
            job.message = format!("{}:\n{error}", tr("Error uploading to print host"));
        }
    }
}
