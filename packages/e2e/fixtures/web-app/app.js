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
  // harness reads it.
  var send = function () {
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
})();
