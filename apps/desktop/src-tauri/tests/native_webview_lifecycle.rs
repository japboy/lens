//! Opt-in real WKWebView ownership regression. Requires a logged-in macOS session.
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("native_webview_lifecycle requires macOS");
}

#[cfg(target_os = "macos")]
mod native {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver, Sender},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

    const CYCLES: usize = 8;
    const DEADLINE: Duration = Duration::from_secs(20);
    const POLL: Duration = Duration::from_millis(20);

    enum Observation {
        DocumentLoaded(usize),
        NativeCallback(usize),
        StreamOpened(usize),
        StreamClosed(usize),
        WindowDestroyed(usize),
        Failed(String),
    }

    enum Phase {
        Ready {
            document: bool,
            stream: bool,
            native_callback: bool,
        },
        Closed {
            window: bool,
            stream: bool,
        },
    }

    fn await_phase(
        events: &Receiver<Observation>,
        cycle: usize,
        mut phase: Phase,
    ) -> Result<(), String> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let observation = events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| format!("cycle {cycle}: lifecycle deadline: {error}"))?;
            match (&mut phase, observation) {
                (_, Observation::Failed(error)) => return Err(error),
                (Phase::Ready { document, .. }, Observation::DocumentLoaded(id)) if id == cycle => {
                    *document = true;
                }
                (
                    Phase::Ready {
                        native_callback, ..
                    },
                    Observation::NativeCallback(id),
                ) if id == cycle => {
                    *native_callback = true;
                }
                (Phase::Ready { stream, .. }, Observation::StreamOpened(id)) if id == cycle => {
                    *stream = true;
                }
                (Phase::Closed { window, .. }, Observation::WindowDestroyed(id)) if id == cycle => {
                    *window = true;
                }
                (Phase::Closed { stream, .. }, Observation::StreamClosed(id)) if id == cycle => {
                    *stream = true;
                }
                _ => return Err(format!("cycle {cycle}: unexpected lifecycle transition")),
            }
            if matches!(
                phase,
                Phase::Ready {
                    document: true,
                    stream: true,
                    native_callback: true
                } | Phase::Closed {
                    window: true,
                    stream: true
                }
            ) {
                return Ok(());
            }
        }
    }

    fn serve(
        mut socket: TcpStream,
        events: Sender<Observation>,
        stopped: Arc<AtomicBool>,
    ) -> Result<(), String> {
        socket
            .set_read_timeout(Some(POLL))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(DEADLINE))
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + DEADLINE;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            if request.len() > 8192 || Instant::now() >= deadline || stopped.load(Ordering::Acquire)
            {
                return Err("fixture request exceeded its bound".into());
            }
            let mut byte = [0];
            match socket.read(&mut byte) {
                Ok(0) if request.is_empty() => return Ok(()),
                Ok(0) => return Err("incomplete fixture request".into()),
                Ok(_) => request.push(byte[0]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let request = String::from_utf8(request).map_err(|e| e.to_string())?;
        let path = request
            .split_whitespace()
            .nth(1)
            .ok_or("missing fixture path")?;
        let (route, cycle) = path.split_once("?cycle=").ok_or("invalid fixture route")?;
        let cycle: usize = cycle.parse().map_err(|_| "invalid fixture cycle")?;
        if !(1..=CYCLES).contains(&cycle) {
            return Err("fixture cycle outside declared range".into());
        }
        if route == "/document" {
            let body = format!("<!doctype html><title>Lifecycle {cycle}</title><script>window.stream = new EventSource('/events?cycle={cycle}');</script>");
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())?;
        } else if route == "/events" {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n: connected\n\n").map_err(|e| e.to_string())?;
            events
                .send(Observation::StreamOpened(cycle))
                .map_err(|e| e.to_string())?;
            loop {
                if stopped.load(Ordering::Acquire) {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "cycle {cycle}: SSE remained owned after window close"
                    ));
                }
                let mut byte = [0];
                match socket.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => return Err("unexpected SSE client payload".into()),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
                    Err(e) => return Err(e.to_string()),
                }
            }
            events
                .send(Observation::StreamClosed(cycle))
                .map_err(|e| e.to_string())?;
        } else {
            return Err("unknown fixture route".into());
        }
        Ok(())
    }

    pub fn run() {
        // A stuck native main loop cannot service handle.exit; bound that failure too.
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(120));
            eprintln!("FAIL: native lifecycle test exceeded its overall deadline");
            std::process::exit(1);
        });
        let succeeded = Arc::new(AtomicBool::new(false));
        let result_flag = succeeded.clone();
        let (events_tx, events_rx) = mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind isolated fixture");
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let server_stop = stopped.clone();
        let server_events = events_tx.clone();
        let server = thread::spawn(move || {
            let mut clients = Vec::new();
            while !server_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        if clients.len() >= CYCLES * 4 {
                            let _ = server_events.send(Observation::Failed(
                                "fixture connection bound exceeded".into(),
                            ));
                            break;
                        }
                        let events = server_events.clone();
                        let stop = server_stop.clone();
                        clients.push(thread::spawn(move || {
                            if let Err(error) = serve(socket, events.clone(), stop) {
                                let _ = events.send(Observation::Failed(error));
                            }
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                    Err(e) => {
                        let _ = server_events.send(Observation::Failed(e.to_string()));
                        break;
                    }
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        let app = tauri::Builder::default()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build isolated real Tauri application");
        let handle = app.handle().clone();
        thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                for cycle in 1..=CYCLES {
                    let app = handle.clone();
                    let events = events_tx.clone();
                    let url = format!("{origin}/document?cycle={cycle}");
                    handle
                        .run_on_main_thread(move || {
                            let loaded = events.clone();
                            let result = WebviewWindowBuilder::new(
                                &app,
                                "lifecycle",
                                WebviewUrl::External(url.parse().unwrap()),
                            )
                            .visible(false)
                            .on_page_load(move |_, payload| {
                                if matches!(
                                    payload.event(),
                                    tauri::webview::PageLoadEvent::Finished
                                ) {
                                    let _ = loaded.send(Observation::DocumentLoaded(cycle));
                                }
                            })
                            .build()
                            .and_then(|window| {
                                let destroyed = events.clone();
                                window.on_window_event(move |event| {
                                    if matches!(event, WindowEvent::Destroyed) {
                                        let _ = destroyed.send(Observation::WindowDestroyed(cycle));
                                    }
                                });
                                // The public callback borrows its native handles. Even doing
                                // nothing here leaked three ObjC retains in runtime-wry 2.11.4.
                                let native_callback = events.clone();
                                window.with_webview(move |_| {
                                    let _ =
                                        native_callback.send(Observation::NativeCallback(cycle));
                                })
                            });
                            if let Err(error) = result {
                                let _ = events.send(Observation::Failed(error.to_string()));
                            }
                        })
                        .map_err(|e| e.to_string())?;
                    await_phase(
                        &events_rx,
                        cycle,
                        Phase::Ready {
                            document: false,
                            stream: false,
                            native_callback: false,
                        },
                    )?;
                    handle
                        .get_webview_window("lifecycle")
                        .ok_or("missing live window")?
                        .close()
                        .map_err(|e| e.to_string())?;
                    await_phase(
                        &events_rx,
                        cycle,
                        Phase::Closed {
                            window: false,
                            stream: false,
                        },
                    )?;
                    println!("PASS cycle {cycle}: document loaded, native window destroyed, SSE released");
                }
                Ok(())
            })();
            stopped.store(true, Ordering::Release);
            let fixture_result = server
                .join()
                .map_err(|_| "fixture server panicked".to_owned())
                .and_then(|()| {
                    // Joining all clients precedes this drain: errors emitted after the
                    // final Closed transition must still fail the complete test.
                    match events_rx.try_iter().next() {
                        Some(Observation::Failed(error)) => Err(error),
                        Some(_) => Err("unexpected observation after final teardown".into()),
                        None => Ok(()),
                    }
                });
            let result = result.and(fixture_result);
            if let Err(error) = &result {
                eprintln!("FAIL: {error}");
            }
            result_flag.store(result.is_ok(), Ordering::Release);
            handle.exit(i32::from(result.is_err()));
        });
        app.run(move |_, event| {
            // Preserve the test result even on runtime versions that lose exit codes.
            if matches!(event, RunEvent::Exit) {
                std::process::exit(i32::from(!succeeded.load(Ordering::Acquire)));
            }
            if let RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}
