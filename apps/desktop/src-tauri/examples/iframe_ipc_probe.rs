//! Disposable macOS/Tauri IPC experiment. Never loaded by the Lens application.
//! The Objective-C method replacement is a test instrument, not a shipping design.
use std::{
    ffi::{c_char, c_void},
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};
use tauri::{http::Response, Manager};

static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static BLOCKED: AtomicUsize = AtomicUsize::new(0);
static EXECUTED: AtomicUsize = AtomicUsize::new(0);

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn class_getInstanceMethod(class: *mut c_void, selector: *mut c_void) -> *mut c_void;
    fn method_getImplementation(method: *mut c_void) -> *mut c_void;
    fn method_setImplementation(method: *mut c_void, imp: *mut c_void) -> *mut c_void;
    fn objc_msgSend();
}

unsafe extern "C" fn guarded_ipc(
    receiver: *mut c_void,
    selector: *mut c_void,
    controller: *mut c_void,
    message: *mut c_void,
) {
    let object_message: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
        std::mem::transmute(objc_msgSend as *const ());
    let bool_message: unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool =
        std::mem::transmute(objc_msgSend as *const ());
    let frame = object_message(message, sel_registerName(c"frameInfo".as_ptr()));
    if !bool_message(frame, sel_registerName(c"isMainFrame".as_ptr())) {
        BLOCKED.fetch_add(1, Ordering::SeqCst);
        println!("NATIVE_SUBFRAME_REJECTED");
        return;
    }
    let original: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(ORIGINAL.load(Ordering::SeqCst));
    original(receiver, selector, controller, message);
}

unsafe fn install_test_guard() -> Result<(), &'static str> {
    let class = objc_getClass(
        c"wry::wkwebview::class::wry_web_view_delegate::WryWebViewDelegate0.55.1".as_ptr(),
    );
    if class.is_null() {
        return Err("Expected Wry 0.55.1 delegate class was not found");
    }
    let method = class_getInstanceMethod(
        class,
        sel_registerName(c"userContentController:didReceiveScriptMessage:".as_ptr()),
    );
    if method.is_null() {
        return Err("Expected Wry IPC method was not found");
    }
    ORIGINAL.store(method_getImplementation(method), Ordering::SeqCst);
    method_setImplementation(method, guarded_ipc as *mut c_void);
    Ok(())
}

const CHILD: &str = r#"<!doctype html><body>Isolated test frame<script>
addEventListener('message', async e => {
  if (e.data?.kind !== 'probe') return;
  if (e.data.packet.transport === 'fetch') {
    const p = e.data.packet;
    try {
      const response = await fetch('ipc://localhost/' + encodeURIComponent(p.cmd), {
        method:'POST',
        headers:{'Content-Type':'application/json','Tauri-Invoke-Key':p.__TAURI_INVOKE_KEY__,'Tauri-Callback':String(p.callback),'Tauri-Error':String(p.error)},
        body:JSON.stringify(p.payload)
      });
      parent.postMessage({kind:'fetch-result',id:p.callback,status:response.status,text:await response.text()}, '*');
    } catch(error) { parent.postMessage({kind:'fetch-result',id:p.callback,error:String(error)}, '*'); }
    return;
  }
  // The correct key is deliberately supplied by the isolated test parent.
  // This tests authority independently of key secrecy. Never do this in Lens.
  try { webkit.messageHandlers.ipc.postMessage(JSON.stringify(e.data.packet)); }
  catch (error) { parent.postMessage({kind:'transport-error',message:String(error)}, '*'); }
});
parent.postMessage({kind:'ready'}, '*');
</script>"#;

fn main() {
    let guarded = std::env::args().any(|arg| arg == "--guard");
    let permit_ipc_fetch = std::env::args().any(|arg| arg == "--permit-ipc-fetch");
    let preview_csp = if permit_ipc_fetch {
        "default-src 'none'; script-src 'unsafe-inline'; connect-src ipc:; sandbox allow-scripts"
    } else {
        "default-src 'none'; script-src 'unsafe-inline'; connect-src 'none'; sandbox allow-scripts"
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: {preview_csp}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                CHILD.len(), CHILD
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    let script = r#"
(() => {
const key = __INVOKE_KEY__;
window.__TAURI_INTERNALS__.postMessage = message => {
  webkit.messageHandlers.ipc.postMessage(JSON.stringify({...message,__TAURI_INVOKE_KEY__:key}));
};
addEventListener('DOMContentLoaded', async () => {
  const rows = [];
  async function test(label, frame, cmd, transport = 'native') {
    const result = await new Promise(resolve => {
      const callback = __TAURI_INTERNALS__.transformCallback(value => resolve({status:'ok',value}), true);
      const error = __TAURI_INTERNALS__.transformCallback(value => resolve({status:'error',value}), true);
      const packet = {cmd, callback, error, payload:{label}, options:{customProtocolIpcBlocked:true}, __TAURI_INVOKE_KEY__:key, transport};
      if (transport === 'fetch') {
        if (!frame) {
          fetch('ipc://localhost/' + encodeURIComponent(cmd), {
            method:'POST',headers:{'Content-Type':'application/json','Tauri-Invoke-Key':key,'Tauri-Callback':String(callback),'Tauri-Error':String(error)},body:JSON.stringify(packet.payload)
          }).then(async response => resolve({status:'fetch-result',value:{status:response.status,text:await response.text()}})).catch(error => resolve({status:'fetch-result',value:{error:String(error)}}));
          return;
        }
        const receive = e => {
          if (e.source === frame.contentWindow && e.data?.kind === 'fetch-result' && e.data.id === callback) {
            removeEventListener('message',receive); resolve({status:'fetch-result',value:e.data});
          }
        };
        addEventListener('message',receive);
      }
      if (frame) frame.contentWindow.postMessage({kind:'probe',packet}, '*');
      else webkit.messageHandlers.ipc.postMessage(JSON.stringify(packet));
      setTimeout(() => resolve({status:'timeout'}), 800);
    });
    rows.push({label,cmd,transport,...result});
  }
  const commands = ['boundary_probe','plugin:app|version','plugin:__TAURI_CHANNEL__|fetch'];
  for (const cmd of commands) await test('parent',null,cmd);
  for (const cmd of commands) await test('parent',null,cmd,'fetch');
  for (const [label,url] of [['custom','boundary-preview://document'],['http','http://ADDRESS/document']]) {
    const frame = document.createElement('iframe');
    frame.sandbox = 'allow-scripts';
    const ready = new Promise(resolve => {
      const receive = e => {
        if (e.source === frame.contentWindow && e.data?.kind === 'ready') {removeEventListener('message',receive);resolve();}
      };
      addEventListener('message',receive);
    });
    frame.src = url; document.body.append(frame); await ready;
    for (const cmd of commands) await test(label,frame,cmd);
    for (const cmd of commands) await test(label,frame,cmd,'fetch');
    frame.remove();
  }
  await __TAURI_INTERNALS__.invoke('boundary_report',{rows});
});
})();
"#.replace("ADDRESS", &address.to_string());
    let app = tauri::Builder::default()
        .invoke_system(script)
        .register_uri_scheme_protocol("boundary-parent", |_, _| {
            Response::builder()
                .header("Content-Type", "text/html")
                .header("Content-Security-Policy", "default-src 'none'; frame-src boundary-preview: http://127.0.0.1:*; script-src 'self'; connect-src ipc:")
                .body(b"<!doctype html><body>Disposable IPC probe</body>".to_vec()).unwrap()
        })
        .register_uri_scheme_protocol("boundary-preview", move |_, _| {
            Response::builder()
                .header("Content-Type", "text/html")
                .header("Content-Security-Policy", preview_csp)
                .body(CHILD.as_bytes().to_vec()).unwrap()
        })
        .invoke_handler(move |invoke| {
            match invoke.message.command() {
                "boundary_probe" => {
                    EXECUTED.fetch_add(1, Ordering::SeqCst);
                    println!("BENIGN_COMMAND_EXECUTED {:?}", invoke.message.payload());
                    invoke.resolver.resolve("benign-success");
                }
                "boundary_report" => {
                    println!("RESULT {:?}", invoke.message.payload());
                    println!("COUNTS guarded={guarded} executed={} blocked={}",EXECUTED.load(Ordering::SeqCst),BLOCKED.load(Ordering::SeqCst));
                    let expected = if guarded {2} else {3};
                    let rows_valid = match invoke.message.payload() {
                        tauri::ipc::InvokeBody::Json(value) => value["rows"].as_array().is_some_and(|rows| {
                            rows.len() == 18 && rows.iter().all(|row| {
                                if row["label"] == "parent" {
                                    if row["transport"] == "native" {
                                        if row["cmd"] == "plugin:__TAURI_CHANNEL__|fetch" {row["value"] == "missing channel id header"} else {row["status"] == "ok"}
                                    } else if row["cmd"] == "plugin:__TAURI_CHANNEL__|fetch" {
                                        row["value"]["text"] == "\"missing channel id header\""
                                    } else {row["value"]["status"] == 200}
                                } else if row["transport"] == "fetch" {
                                    if permit_ipc_fetch {row["value"]["status"] == 500 && row["value"]["text"] == "Origin header is not a valid URL"}
                                    else {row["value"]["error"].as_str().is_some_and(|error| error.starts_with("TypeError"))}
                                } else if guarded {row["status"] == "timeout"}
                                else if row["cmd"] == "plugin:__TAURI_CHANNEL__|fetch" {row["value"] == "missing channel id header"}
                                else if row["label"] == "custom" {row["status"] == "ok"}
                                else {row["status"] == "error" && row["value"].as_str().is_some_and(|value| value.contains("not allowed"))}
                            })
                        }),
                        _ => false,
                    };
                    let passed = rows_valid && EXECUTED.load(Ordering::SeqCst) == expected && (!guarded || BLOCKED.load(Ordering::SeqCst) == 6);
                    println!("ASSERTIONS_PASSED={passed}");
                    let app = invoke.message.webview().app_handle().clone();
                    invoke.resolver.resolve(passed);
                    app.exit(if passed {0} else {2});
                }
                _ => return false,
            }
            true
        })
        .setup(move |app| {
            tauri::WebviewWindowBuilder::new(app,"lens-overlay",tauri::WebviewUrl::External("boundary-parent://document".parse()?))
                .title("Disposable Lens IPC probe")
                .visible(false)
                .build()?;
            if guarded {
                if let Err(message) = unsafe {install_test_guard()} {
                    eprintln!("PROBE_SETUP_FAILED: {message}");
                    // Do not unwind across AppKit's callback boundary or produce a crash dialog.
                    std::process::exit(4);
                }
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {std::thread::sleep(std::time::Duration::from_secs(20));println!("PROBE_TIMEOUT");handle.exit(3);});
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build isolated Tauri probe");
    let code = app.run_return(|_, _| {});
    std::process::exit(code);
}
