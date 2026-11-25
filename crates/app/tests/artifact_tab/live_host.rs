use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};

pub const TOKEN: &str = "viewer-fixture-token-1567-abcdefgh";
pub const ARTIFACT: &str = "artifact:test-sales";

pub struct Host {
    dir: tempfile::TempDir,
    update: mpsc::Sender<()>,
}

impl Host {
    pub fn start(thread_id: &'static str) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("cgx-artifact-")
            .tempdir_in("/tmp")?;
        let (first, second) = (workbook(dir.path(), 1)?, workbook(dir.path(), 2)?);
        let listener = UnixListener::bind(dir.path().join("host.sock"))?;
        let (update, changes) = mpsc::channel();
        thread::spawn(move || serve(listener, thread_id, first, second, changes));
        Ok(Self { dir, update })
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn commit(&self) -> Result<()> {
        Ok(self.update.send(())?)
    }
}

fn serve(
    listener: UnixListener,
    thread_id: &str,
    first: PathBuf,
    second: PathBuf,
    changes: mpsc::Receiver<()>,
) -> Result<()> {
    let (mut socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(Duration::from_secs(20)))?;
    let ping = request(&mut socket, "host.ping")?;
    answer(
        &mut socket,
        &ping,
        json!({"capabilities":["native-snapshot-v1"]}),
    )?;
    let open = request(&mut socket, "subscriptions.open")?;
    let id = open["params"]["subscriptionId"]
        .as_str()
        .context("subscription id")?;
    ensure!(open["params"]["threadId"] == thread_id && open["params"]["artifactRef"] == ARTIFACT);
    answer(
        &mut socket,
        &open,
        json!({"subscriptionId":id,"document":{
            "kind":"spreadsheet","title":"Quarterly Sales","stateVersion":1,
            "checkpointStateVersion":1,"checkpointCrdtStateBase64":"AAA=",
            "initialArtifactProtoBase64":"","updates":[]
        }}),
    )?;
    snapshot(&mut socket, &first, thread_id, id, 1)?;
    changes.recv_timeout(Duration::from_secs(20))?;
    send(
        &mut socket,
        json!({"schema":"openai.artifact-session.host-event/v1",
        "subscriptionId":id,"event":{"type":"artifact-session-update-committed",
        "stateVersion":2,"update":{"baseStateVersion":1,"stateVersion":2,
        "crdtUpdateBase64":"AAA=","originId":"test-model","source":"model","updateId":"update-2"}}}),
    )?;
    snapshot(&mut socket, &second, thread_id, id, 2)?;
    // Keep the connection open until the test drops the tab or host.
    let _ = changes.recv_timeout(Duration::from_secs(20));
    Ok(())
}

fn snapshot(
    socket: &mut UnixStream,
    path: &Path,
    thread_id: &str,
    id: &str,
    version: u64,
) -> Result<()> {
    let req = request(socket, "outputs.publishNativeSnapshot")?;
    ensure!(req["params"] == json!({"threadId":thread_id,"artifactRef":ARTIFACT}));
    send(
        socket,
        json!({"schema":"openai.artifact-session.host-call/v1","id":"flush",
        "method":"viewer.flush","params":{"subscriptionId":id}}),
    )?;
    ensure!(
        recv(socket)?
            == json!({"schema":"openai.artifact-session.host-call-response/v1",
        "id":"flush","ok":true,"result":{"flushed":true}})
    );
    answer(
        socket,
        &req,
        json!({"artifactRef":ARTIFACT,"artifactType":"spreadsheet",
        "stateVersion":version,
        "binding":{"fileName":"workbook.xlsx"},"format":"xlsx","value":{"path":path},
        "contentType":"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "byteLength":fs::metadata(path)?.len()}),
    )
}

fn request(socket: &mut UnixStream, method: &str) -> Result<Value> {
    let frame = recv(socket)?;
    ensure!(frame["schema"] == "openai.artifact-session.host-request/v1");
    ensure!(frame["token"] == TOKEN && frame["method"] == method);
    Ok(frame)
}

fn answer(socket: &mut UnixStream, req: &Value, result: Value) -> Result<()> {
    send(
        socket,
        json!({"schema":"openai.artifact-session.host-response/v1",
        "id":req["id"],"ok":true,"result":result}),
    )
}

fn send(socket: &mut UnixStream, frame: Value) -> Result<()> {
    let bytes = serde_json::to_vec(&frame)?;
    socket.write_all(&(u32::try_from(bytes.len())?).to_be_bytes())?;
    Ok(socket.write_all(&bytes)?)
}

fn recv(socket: &mut UnixStream) -> Result<Value> {
    let mut size = [0; 4];
    socket.read_exact(&mut size)?;
    let mut bytes = vec![0; u32::from_be_bytes(size) as usize];
    socket.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn workbook(root: &Path, version: u8) -> Result<PathBuf> {
    let dir = root.join(version.to_string());
    fs::create_dir_all(dir.join("_rels"))?;
    fs::create_dir_all(dir.join("xl/_rels"))?;
    fs::create_dir_all(dir.join("xl/worksheets"))?;
    fs::write(
        dir.join("[Content_Types].xml"),
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
    )?;
    fs::write(
        dir.join("_rels/.rels"),
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    )?;
    fs::write(
        dir.join("xl/workbook.xml"),
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sales" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
    )?;
    fs::write(
        dir.join("xl/_rels/workbook.xml.rels"),
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
    )?;
    let sales = if version == 1 { "1200" } else { "98400" };
    fs::write(
        dir.join("xl/worksheets/sheet1.xml"),
        format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B2"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Quarter</t></is></c><c r="B1" t="inlineStr"><is><t>Sales</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Q4</t></is></c><c r="B2"><v>{sales}</v></c></row></sheetData></worksheet>"#
        ),
    )?;
    let output = Command::new("/usr/bin/zip")
        .current_dir(&dir)
        .args(["-qr", "workbook.xlsx", "[Content_Types].xml", "_rels", "xl"])
        .output()?;
    ensure!(output.status.success(), "zip failed");
    Ok(dir.join("workbook.xlsx"))
}
