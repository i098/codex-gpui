import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { openPage, navigate } from "./chrome.mjs";

const page = await openPage();
try {
  await page.send("Page.enable");
  await page.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 832, deviceScaleFactor: 0, mobile: false });
  await page.send("Page.navigate", { url: `${process.env.R1_SERVER || "http://127.0.0.1:4517"}/?theme=dark` });
  const url = await page.evaluate("location.href");
  await navigate(page, "/settings");
  let ready = false;
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline) {
    ready = await page.evaluate(`document.body.innerText.includes("Default permissions") && [...document.querySelectorAll("h1,h2")].some(e => e.textContent.trim() === "General")`);
    if (ready) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.equal(ready, true, "The Settings controls did not appear");
  assert.equal(await page.evaluate("location.href"), url);
  if (process.env.R1_OUT) {
    fs.mkdirSync(process.env.R1_OUT, { recursive: true });
    const { data } = await page.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(process.env.R1_OUT, "navigation-settings.png"), Buffer.from(data, "base64"));
  }
  console.log("navigation: Settings controls appeared without changing the browser URL");
} finally {
  await page.close();
}
