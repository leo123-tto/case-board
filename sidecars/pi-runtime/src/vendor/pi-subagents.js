// Keep the upstream package behind a JavaScript boundary. pi-subagents intentionally
// publishes TypeScript source for Pi's runtime loader, but CaseBoard's stricter tsc
// configuration must not type-check the package's internal implementation.
export { default } from "pi-subagents";
