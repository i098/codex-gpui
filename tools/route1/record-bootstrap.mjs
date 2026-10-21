import fs from "node:fs";
import { connect } from "../capture/cdp.mjs";

const output = process.argv[2];
if (!output) throw new Error("Usage: node tools/route1/record-bootstrap.mjs <scratch-oracle.json>");
const oracle = JSON.parse(fs.readFileSync(output, "utf8"));
const modulePath = process.env.R1_RPC_MODULE || "./assets/rpc-f6f57972be0d.js";
const page = await connect();
try {
  const { version, records } = await page.evaluate(`(async () => {
    const services = (await import(${JSON.stringify(modulePath)})).appServices;
    const snapshot = await services.threadReadState.open(() => {});
    try {
      if (snapshot.status !== "ready" || snapshot.identity?.kind !== "execution-storage")
        throw new Error("Use an owned, credential-free app-server profile with requires_openai_auth=false");
      const { session, ...state } = snapshot;
      return {
        version: electronBridge.getSentryInitOptions().appVersion,
        records: {
          "svc threadReadState.getExecutionHostKeys []": { value: await services.threadReadState.getExecutionHostKeys(), fn: false, calls: [] },
          'svc threadReadState.open ["__fn",null]': { value: state, fn: false, calls: [] },
          "svc startup.whenReady []": { value: (await services.startup.whenReady()) ?? null, fn: false, calls: [] }
        }
      };
    } finally {
      await snapshot.session?.unsubscribe();
    }
  })()`);
  fs.writeFileSync(output, `${JSON.stringify({ ...oracle, ...records }, null, 1)}\n`);
  console.log(`Recorded ${Object.keys(records).length} bootstrap responses from ${version} into ${output}`);
} finally {
  page.close();
}
