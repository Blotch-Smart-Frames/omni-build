// The E2E web app's report: what the page finds on launch, as one line of
// JSON on the console. The WebView shell forwards it to logcat on Android
// and to the unified log on iOS, where the harness reads it. Plain ES5 with
// XMLHttpRequest, since Android 5.0's own WebView is Chrome 37: no fetch,
// arrow functions or template literals.
(function () {
  var report = {
    // From index.html: the payload's page, not the shell's placeholder.
    title: document.title,
    // The origin the shell serves the payload from.
    origin: location.origin,
  };

  try {
    localStorage.setItem('omni-build-e2e', 'works');
    report.storage = localStorage.getItem('omni-build-e2e');
  } catch (error) {
    report.storage = String(error);
  }

  // On screen, for the harness's screenshot, and on the console, where the
  // harness reads it, once the file and the native module have answered.
  var waiting = 2;
  var send = function () {
    waiting -= 1;
    if (waiting > 0) {
      return;
    }
    var shown = document.getElementById('report');
    shown.textContent = JSON.stringify(report, null, 2);
    console.log(JSON.stringify(report));
  };

  // A file beside the page, read as a web app reads its data: what the shell
  // serves for a relative URL.
  var request = new XMLHttpRequest();
  request.onload = function () {
    report.message =
      request.status === 200 ? request.responseText : 'HTTP ' + request.status;
    send();
  };
  request.onerror = function () {
    report.message = 'the request failed';
    send();
  };
  request.open('GET', 'message.txt');
  request.send();

  // The native module the app has, Echo, called as a web app calls one:
  // through window.omniBuild, which the shell gives each page of the
  // payload. What it answers, then how a method it hasn't fails.
  if (!window.omniBuild) {
    report.modules = 'no window.omniBuild';
    send();
    return;
  }
  report.modules = window.omniBuild.modules.join(', ');
  window.omniBuild
    .call('Echo', 'echo', { text: 'hello' })
    .then(function (answer) {
      report.module = JSON.stringify(answer);
      return window.omniBuild.call('Echo', 'shout');
    })
    .then(
      function () {
        report.moduleError = 'none';
      },
      function (error) {
        report.moduleError = error.message;
      }
    )
    .then(send);
})();
