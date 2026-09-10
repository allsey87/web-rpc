// Drives an extracted endpoint module on behalf of a Rust test. Nothing here knows which
// trait is under test: the Rust side names the module, the class, the method and the
// arguments, and supplies handler tables as source.

const modules = new Map();

/** Import a module the test runner serves. It comes as octet-stream, so it goes via a blob. */
async function load(path) {
  let module = modules.get(path);
  if (!module) {
    module = fetch(path)
      .then((response) => response.text())
      .then((source) => import(blobUrl(source)));
    modules.set(path, module);
  }
  return module;
}

function blobUrl(source) {
  return URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
}

/**
 * Construct `className` from the module at `path` over `endpoint`. `handlers` is the source
 * of an object literal, which may refer to `log`: a plain object handed back beside the
 * instance for the test to inspect through `probe`.
 */
export async function open(path, className, endpoint, handlers) {
  const module = await load(path);
  return construct(module, className, endpoint, handlers);
}

function construct(module, className, endpoint, handlers) {
  const log = {};
  const options = { endpoint };
  if (handlers) options.handlers = new Function("log", `return (${handlers});`)(log);
  return { instance: new module[className](options), log };
}

export function call(handle, method, args) {
  return handle.instance[method](...args);
}

/** Call and abort at once; the name of the rejection, or "resolved". */
export async function abort(handle, method, args) {
  const pending = handle.instance[method](...args);
  pending.abort();
  try {
    await pending;
    return "resolved";
  } catch (thrown) {
    return thrown.name;
  }
}

/** Subscribe to a stream and collect its items until it ends. */
export async function subscribe(handle, method, args) {
  const items = [];
  await handle.instance[method](...args, (item) => {
    items.push(item);
  }).done;
  return items;
}

export function close(handle) {
  handle.instance.close();
}

export function probe(handle) {
  return { ...handle.log };
}

/** Call, then close the endpoint while the call is pending; the name of the rejection. */
export async function callThenClose(handle, method, args) {
  const pending = handle.instance[method](...args);
  handle.instance.close();
  try {
    await pending;
    return "resolved";
  } catch (thrown) {
    return thrown.name;
  }
}

/**
 * A client over a `Worker` that serves `serviceClass` from the module at `servicePath` with
 * `handlers`, or over a worker whose script throws if `servicePath` is null. The client is
 * created with the worker, so it sees every event the worker fires.
 */
export async function openWorker(clientPath, clientClass, servicePath, serviceClass, handlers) {
  const module = await load(clientPath);
  let body = "throw new Error('boom');";
  if (servicePath) {
    const source = await (await fetch(servicePath)).text();
    body = `${source}\nconst log = {};\nnew ${serviceClass}({ endpoint: self, handlers: (${handlers}) });\n`;
  }
  // The test owns the worker; the endpoint is only handed the transport.
  const worker = new Worker(blobUrl(body), { type: "module" });
  return { ...construct(module, clientClass, worker), worker };
}

export function terminate(handle) {
  handle.worker.terminate();
}
