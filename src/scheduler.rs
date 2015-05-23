// The MIT License (MIT)

// Copyright (c) 2015 Rustcc Developers

// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:

// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
// COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::sync::{Mutex, Once, ONCE_INIT};
use std::mem;
use std::cell::UnsafeCell;
use std::io;
#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;
#[cfg(target_os = "linux")]
use std::convert::From;
use std::sync::atomic::{ATOMIC_BOOL_INIT, AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::collections::VecDeque;

use coroutine::{State, Handle, Coroutine, Options};

use deque::{BufferPool, Stealer, Worker, Stolen};

use mio::{EventLoop, Evented, Handler, Token, ReadHint, Interest, PollOpt};
use mio::util::Slab;
#[cfg(target_os = "linux")]
use mio::Io;

static mut THREAD_HANDLES: *const Mutex<Vec<(Sender<SchedMessage>, Stealer<Handle>)>> =
    0 as *const Mutex<Vec<(Sender<SchedMessage>, Stealer<Handle>)>>;
static THREAD_HANDLES_ONCE: Once = ONCE_INIT;
static SCHEDULER_HAS_STARTED: AtomicBool = ATOMIC_BOOL_INIT;

fn schedulers() -> &'static Mutex<Vec<(Sender<SchedMessage>, Stealer<Handle>)>> {
    unsafe {
        THREAD_HANDLES_ONCE.call_once(|| {
            let handles: Box<Mutex<Vec<(Sender<SchedMessage>, Stealer<Handle>)>>> =
                Box::new(Mutex::new(Vec::new()));

            THREAD_HANDLES = mem::transmute(handles);
        });

        & *THREAD_HANDLES
    }
}

thread_local!(static SCHEDULER: UnsafeCell<Scheduler> = UnsafeCell::new(Scheduler::new()));

pub enum SchedMessage {
    NewNeighbor(Sender<SchedMessage>, Stealer<Handle>),
    Shutdown,
}

const MAX_PRIVATE_WORK_NUM: usize = 10;

pub struct Scheduler {
    workqueue: Worker<Handle>,
    workstealer: Stealer<Handle>,

    commchannel: Receiver<SchedMessage>,

    neighbors: Vec<(Sender<SchedMessage>, Stealer<Handle>)>,

    eventloop: EventLoop<SchedulerHandler>,
    handler: SchedulerHandler,

    private_work: VecDeque<Handle>,
}

impl Scheduler {

    fn new() -> Scheduler {
        let bufpool = BufferPool::new();
        let (worker, stealer) = bufpool.deque();

        let (tx, rx) = channel();

        let scheds = schedulers();
        let mut guard = scheds.lock().unwrap();

        for &(ref rtx, _) in guard.iter() {
            let _ = rtx.send(SchedMessage::NewNeighbor(tx.clone(), stealer.clone()));
        }

        let neighbors = guard.clone();
        guard.push((tx, stealer.clone()));

        Scheduler {
            workqueue: worker,
            workstealer: stealer,

            commchannel: rx,

            neighbors: neighbors,

            eventloop: EventLoop::new().unwrap(),
            handler: SchedulerHandler::new(),

            private_work: VecDeque::new(),
        }
    }

    pub fn current() -> &'static mut Scheduler {
        SCHEDULER.with(|s| unsafe {
            &mut *s.get()
        })
    }

    pub fn spawn<F>(f: F)
            where F: FnOnce() + Send + 'static {
        let coro = Coroutine::spawn(f);
        Scheduler::current().ready(coro);

        Coroutine::sched();
    }

    pub fn spawn_opts<F>(f: F, opt: Options)
            where F: FnOnce() + Send + 'static {
        let coro = Coroutine::spawn_opts(f, opt);
        Scheduler::current().ready(coro);

        Coroutine::sched();
    }

    pub fn ready(&mut self, work: Handle) {
        if self.private_work.len() >= MAX_PRIVATE_WORK_NUM {
            self.workqueue.push(work);
        } else {
            self.private_work.push_back(work);
        }
    }

    pub fn run<F>(f: F, threads: usize)
            where F: FnOnce() + Send + 'static {

        assert!(threads >= 1, "Threads must >= 1");
        if SCHEDULER_HAS_STARTED.compare_and_swap(false, true, Ordering::SeqCst) != false {
            panic!("Schedulers are already running!");
        }

        // Start worker threads first
        let counter = Arc::new(AtomicUsize::new(0));
        for tid in 0..threads - 1 {
            let counter = counter.clone();
            thread::Builder::new().name(format!("Thread {}", tid)).spawn(move|| {
                let current = Scheduler::current();
                counter.fetch_add(1, Ordering::SeqCst);
                current.schedule();
            }).unwrap();
        }

        while counter.load(Ordering::SeqCst) != threads - 1 {}

        Scheduler::spawn(|| {
            struct Guard;

            // Send Shutdown to all schedulers
            impl Drop for Guard {
                fn drop(&mut self) {
                    let guard = match schedulers().lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner()
                    };

                    for &(ref chan, _) in guard.iter() {
                        let _ = chan.send(SchedMessage::Shutdown);
                    }
                }
            }

            let _guard = Guard;

            f();
        });

        Scheduler::current().schedule();

        SCHEDULER_HAS_STARTED.store(false, Ordering::SeqCst);
    }

    fn resume_coroutine(&mut self, work: Handle) {
        match work.state() {
            State::Suspended | State::Blocked => {
                debug!("Resuming Coroutine: {:?}", work);

                if let Err(err) = work.resume() {
                    let msg = match err.downcast_ref::<&'static str>() {
                        Some(s) => *s,
                        None => match err.downcast_ref::<String>() {
                            Some(s) => &s[..],
                            None => "Box<Any>",
                        }
                    };

                    error!("Coroutine panicked! {:?}", msg);
                }

                match work.state() {
                    State::Normal | State::Running => {
                        unreachable!();
                    },
                    State::Suspended => {
                        debug!("Coroutine suspended, going to be resumed next round");
                        self.ready(work);
                    },
                    State::Blocked => {
                        debug!("Coroutine blocked, maybe waiting for I/O");
                    },
                    State::Finished | State::Panicked => {
                        debug!("Coroutine state: {:?}, will not be resumed automatically", work.state());
                    }
                }
            },
            _ => {
                error!("Trying to resume coroutine {:?}, but its state is {:?}",
                       work, work.state());
            }
        }
    }

    fn schedule(&mut self) {
        loop {
            match self.commchannel.try_recv() {
                Ok(SchedMessage::NewNeighbor(tx, st)) => {
                    self.neighbors.push((tx, st));
                },
                Ok(SchedMessage::Shutdown) => {
                    info!("Shutting down");
                    break;
                },
                Err(TryRecvError::Empty) => {},
                _ => panic!("Receiving from channel: Unknown message")
            }

            if !self.handler.slabs.is_empty() {
                self.eventloop.run_once(&mut self.handler).unwrap();
            }

            debug!("Trying to resume all ready coroutines: {:?}", thread::current().name());
            // Run all ready coroutines
            let mut need_steal = true;
            // while let Some(work) = self.workqueue.pop() {
            // while let Stolen::Data(work) = self.workstealer.steal() {
            //     need_steal = false;
            //     self.resume_coroutine(work);
            // }

            while let Some(work) = self.private_work.pop_front() {
                need_steal = false;
                self.resume_coroutine(work);
            }

            if need_steal {
                if let Stolen::Data(work) = self.workstealer.steal() {
                    need_steal = false;
                    self.resume_coroutine(work);
                }
            }

            if !need_steal || !self.handler.slabs.is_empty() {
                continue;
            }

            debug!("Trying to steal from neighbors: {:?}", thread::current().name());

            // if self.neighbors.len() > 0 {
            //     let neighbor_idx = ::rand::random::<usize>() % self.neighbors.len();
            //     let stolen = {
            //         let &(_, ref neighbor_stealer) = &self.neighbors[neighbor_idx];
            //         neighbor_stealer.steal()
            //     };

            //     if let Stolen::Data(coro) = stolen {
            //         self.resume_coroutine(coro);
            //         continue;
            //     }
            // }
            let mut has_stolen = false;
            let stolen_works = self.neighbors.iter()
                    .filter_map(|&(_, ref st)|
                        if let Stolen::Data(w) = st.steal() {
                            Some(w)
                        } else {
                            None
                        })
                    .collect::<Vec<Handle>>();
            for work in stolen_works.into_iter() {
                has_stolen = true;
                self.resume_coroutine(work);
            }

            if !has_stolen {
                thread::sleep_ms(100);
            }
        }
    }

    // fn resume(&mut self, handle: Handle) {
    //     self.workqueue.push(handle);
    // }
}

const MAX_TOKEN_NUM: usize = 102400;
impl SchedulerHandler {
    fn new() -> SchedulerHandler {
        SchedulerHandler {
            // slabs: Slab::new_starting_at(Token(1), MAX_TOKEN_NUM),
            slabs: Slab::new(MAX_TOKEN_NUM),
        }
    }
}

#[cfg(any(target_os = "linux",
          target_os = "android"))]
impl Scheduler {
    pub fn wait_event<E: Evented + AsRawFd>(&mut self, fd: &E, interest: Interest) -> io::Result<()> {
        let token = self.handler.slabs.insert((Coroutine::current(), From::from(fd.as_raw_fd()))).unwrap();
        try!(self.eventloop.register_opt(fd, token, interest,
                                         PollOpt::level()|PollOpt::oneshot()));

        debug!("wait_event: Blocked current Coroutine ...; token={:?}", token);
        Coroutine::block();
        debug!("wait_event: Waked up; token={:?}", token);

        Ok(())
    }
}

#[cfg(any(target_os = "linux",
          target_os = "android"))]
struct SchedulerHandler {
    slabs: Slab<(Handle, Io)>,
}

#[cfg(any(target_os = "linux",
          target_os = "android"))]
impl Handler for SchedulerHandler {
    type Timeout = ();
    type Message = ();

    fn writable(&mut self, event_loop: &mut EventLoop<Self>, token: Token) {

        debug!("In writable, token {:?}", token);

        match self.slabs.remove(token) {
            Some((hdl, fd)) => {
                // Linux EPoll needs to explicit EPOLL_CTL_DEL the fd
                event_loop.deregister(&fd).unwrap();
                mem::forget(fd);
                Scheduler::current().ready(hdl);
            },
            None => {
                warn!("No coroutine is waiting on writable {:?}", token);
            }
        }

    }

    fn readable(&mut self, event_loop: &mut EventLoop<Self>, token: Token, hint: ReadHint) {

        debug!("In readable, token {:?}, hint {:?}", token, hint);

        match self.slabs.remove(token) {
            Some((hdl, fd)) => {
                // Linux EPoll needs to explicit EPOLL_CTL_DEL the fd
                event_loop.deregister(&fd).unwrap();
                mem::forget(fd);
                Scheduler::current().ready(hdl);
            },
            None => {
                warn!("No coroutine is waiting on readable {:?}", token);
            }
        }

    }
}

#[cfg(any(target_os = "macos",
          target_os = "freebsd",
          target_os = "dragonfly",
          target_os = "ios",
          target_os = "bitrig",
          target_os = "openbsd"))]
impl Scheduler {
    pub fn wait_event<E: Evented>(&mut self, fd: &E, interest: Interest) -> io::Result<()> {
        let token = self.handler.slabs.insert(Coroutine::current()).unwrap();
        try!(self.eventloop.register_opt(fd, token, interest,
                                         PollOpt::level()|PollOpt::oneshot()));

        debug!("wait_event: Blocked current Coroutine ...; token={:?}", token);
        Coroutine::block();
        debug!("wait_event: Waked up; token={:?}", token);

        Ok(())
    }
}

#[cfg(any(target_os = "macos",
          target_os = "freebsd",
          target_os = "dragonfly",
          target_os = "ios",
          target_os = "bitrig",
          target_os = "openbsd"))]
struct SchedulerHandler {
    slabs: Slab<Handle>,
}

#[cfg(any(target_os = "macos",
          target_os = "freebsd",
          target_os = "dragonfly",
          target_os = "ios",
          target_os = "bitrig",
          target_os = "openbsd"))]
impl Handler for SchedulerHandler {
    type Timeout = ();
    type Message = ();

    fn writable(&mut self, _: &mut EventLoop<Self>, token: Token) {

        debug!("In writable, token {:?}", token);

        match self.slabs.remove(token) {
            Some(hdl) => {
                Scheduler::current().ready(hdl);
            },
            None => {
                warn!("No coroutine is waiting on writable {:?}", token);
            }
        }

    }

    fn readable(&mut self, _: &mut EventLoop<Self>, token: Token, hint: ReadHint) {

        debug!("In readable, token {:?}, hint {:?}", token, hint);

        match self.slabs.remove(token) {
            Some(hdl) => {
                Scheduler::current().ready(hdl);
            },
            None => {
                warn!("No coroutine is waiting on readable {:?}", token);
            }
        }

    }
}
