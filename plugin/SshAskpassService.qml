import QtQuick
import Quickshell
import Quickshell.Io

Item {
  id: root

  readonly property int protocolVersion: 2
  readonly property var modes: ["passphrase", "confirm", "notify"]
  // The binary cuts prompts to 2000 bytes, so a longer request isn't from it.
  readonly property int maxRequestBytes: 16384
  readonly property int maxPromptChars: 2000
  property var activeConn: null
  property string activeMode: ""

  function reply(conn, response) {
    conn.write(JSON.stringify(response) + "\n")
    conn.flush()
    conn.connected = false
  }

  function handleRequest(conn, line) {
    if (conn.handled) return
    conn.handled = true

    if (line.length > maxRequestBytes) {
      reply(conn, { result: "error", message: "request too large" })
      return
    }
    var req
    try {
      req = JSON.parse(line)
    } catch (e) {
      reply(conn, { result: "error", message: "invalid JSON" })
      return
    }
    if (!req || req.v !== protocolVersion || modes.indexOf(req.mode) === -1
        || (req.prompt !== undefined && typeof req.prompt !== "string")) {
      reply(conn, { result: "error", message: "unsupported request" })
      return
    }
    if (req.prompt && req.prompt.length > maxPromptChars) {
      reply(conn, { result: "error", message: "prompt too long" })
      return
    }
    if (activeConn) {
      reply(conn, { result: "busy" })
      return
    }
    activeConn = conn
    activeMode = req.mode
    dialog.open(req)
  }

  function finish(result, password) {
    var conn = activeConn
    var mode = activeMode
    activeConn = null
    activeMode = ""
    if (!conn) return
    var response = { result: result }
    if (result === "ok" && mode === "passphrase") {
      try {
        response.password = encodeURIComponent(password)
      } catch (e) {
        response = { result: "error", message: "Password is not valid Unicode" }
      }
    }
    password = ""
    reply(conn, response)
    response = null
  }

  SshAskpassDialog {
    id: dialog
    onFinished: function(result, password) { root.finish(result, password) }
  }

  SocketServer {
    active: true
    path: Quickshell.env("SSH_ASKPASS_OMARCHY_SOCKET") || (Quickshell.env("XDG_RUNTIME_DIR") + "/ssh-askpass-omarchy.sock")
    handler: Socket {
      id: conn
      property bool handled: false

      onConnectionStateChanged: {
        if (!connected && root.activeConn === conn) {
          root.activeConn = null
          root.activeMode = ""
          dialog.close()
        }
      }

      parser: SplitParser {
        onRead: function(line) { root.handleRequest(conn, line) }
      }
    }
  }
}
