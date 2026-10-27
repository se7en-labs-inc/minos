const { Elm } = require('../tests.js');
const app = Elm.Tests.init({ flags: null });
const timeout = setTimeout(() => { console.error('Elm tests timed out'); process.exit(1); }, 5000);
app.ports.results.subscribe(results => {
  clearTimeout(timeout);
  for (const result of results) console.log(`${result.passed ? 'PASS' : 'FAIL'} ${result.name}`);
  process.exitCode = results.every(result => result.passed) ? 0 : 1;
});
