import assert from "node:assert/strict";
import { isCaptureRequest } from "./chrome.mjs";

const request = (url, type = "Document", loaderId = "page") => ({ type, loaderId, request: { url } });
const runner = "https://cdn.platform.openai.com/deployments/dil/v14/runner.html";
assert.equal(isCaptureRequest(request(runner)), false);
assert.equal(isCaptureRequest(request(`${runner}?session=fixture`)), false);
assert.equal(isCaptureRequest(request(runner, "Script")), true);
assert.equal(isCaptureRequest(request("https://cdn.platform.openai.com/deployments/dil/v14/app.js")), true);
assert.equal(isCaptureRequest(request("https://cdn.platform.openai.com.example.org/deployments/dil/v14/runner.html")), true);
assert.equal(isCaptureRequest(request("http://127.0.0.1:35583/assets/app.js", "Script")), true);
assert.equal(isCaptureRequest(request("http://127.0.0.1:35583/__r1/events", "EventSource")), false);
assert.equal(isCaptureRequest(request("http://127.0.0.1:35583/worker.js", "Script", "")), false);
console.log("network idle: only the background runner document is excluded");
