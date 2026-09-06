// CowboyDevBridge — DEBUG-only headless WebKit inspection for the iOS shell.
//
// The iOS Simulator cannot be inspected headlessly through
// ios_webkit_debug_proxy, so simulator automation talks to this loopback HTTP
// server instead. It evaluates JavaScript in the shell's one WKWebView and is
// deliberately absent from release builds. See tools/cowboysim.sh.

#if DEBUG && targetEnvironment(simulator)
import Foundation
import Network
import WebKit

@objc(CowboyDevBridge) public final class CowboyDevBridge: NSObject {
  private static var shared: CowboyDevBridge?
  private weak var webView: WKWebView?
  private var listener: NWListener?
  private var simulatorID = ""

  @objc public static func installOnWebView(_ webView: WKWebView) {
    guard ProcessInfo.processInfo.environment["COWBOY_SIM_BRIDGE"] == "1" else { return }
    if shared == nil { shared = CowboyDevBridge() }
    shared?.webView = webView
    shared?.startIfNeeded()
  }

  private func startIfNeeded() {
    guard listener == nil else { return }
    do {
      let environment = ProcessInfo.processInfo.environment
      guard let identity = environment["SIMULATOR_UDID"], !identity.isEmpty,
        let port = UInt16(environment["COWBOY_SIM_DEVPORT"] ?? "4171"),
        port > 1023
      else { return }
      simulatorID = identity
      let params = NWParameters.tcp
      params.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!)
      // requiredLocalEndpoint already supplies the port. Passing it again as
      // NWListener's `on` argument is rejected by Network.framework (EINVAL).
      let listener = try NWListener(using: params)
      listener.newConnectionHandler = { [weak self] connection in
        self?.handle(connection)
      }
      listener.start(queue: .global(qos: .utility))
      self.listener = listener
      NSLog("[CowboyDevBridge] eval server on 127.0.0.1:\(port) (opt-in DEBUG Simulator)")
    } catch {
      NSLog("[CowboyDevBridge] failed to start: \(error)")
    }
  }

  private func handle(_ connection: NWConnection) {
    connection.start(queue: .global(qos: .utility))
    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 25) {
      connection.cancel()
    }
    receive(connection, buffered: Data())
  }

  private func receive(_ connection: NWConnection, buffered: Data) {
    connection.receive(minimumIncompleteLength: 1, maximumLength: 1 << 20) {
      [weak self] data, _, complete, error in
      guard let self, let data, error == nil else {
        connection.cancel()
        return
      }
      let bytes = buffered + data
      guard bytes.count <= 1 << 20 else { connection.cancel(); return }
      guard let boundary = bytes.range(of: Data("\r\n\r\n".utf8)) else {
        if complete { connection.cancel() } else { self.receive(connection, buffered: bytes) }
        return
      }
      guard let headers = String(data: bytes[..<boundary.lowerBound], encoding: .utf8) else {
        connection.cancel(); return
      }
      let lines = headers.components(separatedBy: "\r\n")
      let requestLine = lines.first ?? ""
      let parts = requestLine.split(separator: " ")
      let path = parts.count > 1 ? String(parts[1]) : "/"
      var fields: [String: String] = [:]
      for line in lines.dropFirst() {
        let pair = line.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
        guard pair.count == 2 else { connection.cancel(); return }
        let key = pair[0].lowercased()
        guard fields[key] == nil else { connection.cancel(); return }
        fields[key] = pair[1].trimmingCharacters(in: .whitespaces)
      }
      // A caller must name this exact Simulator. Browser-origin requests,
      // preflights and chunked/ambiguous requests cannot execute JavaScript.
      guard fields["origin"] == nil, fields["transfer-encoding"] == nil,
        fields["x-cowboy-simulator"] == self.simulatorID,
        let length = Int(fields["content-length"] ?? "0"), length >= 0, length <= 1 << 20,
        parts.first == (path == "/ping" ? "GET" : "POST")
      else { self.respond(connection, "forbidden", status: "403 Forbidden"); return }
      let payload = bytes[boundary.upperBound...]
      if payload.count < length {
        if complete { connection.cancel() } else { self.receive(connection, buffered: bytes) }
        return
      }
      guard payload.count == length, let body = String(data: payload, encoding: .utf8) else {
        connection.cancel(); return
      }
      if path == "/ping" {
        respond(connection, "ok")
        return
      }
      if path == "/aeval" {
        let source = body.isEmpty ? "return undefined" : body
        DispatchQueue.main.async {
          guard let webView = self.webView else {
            self.respond(connection, "ERR: no webview")
            return
          }
          webView.callAsyncJavaScript(
            source,
            arguments: [:],
            in: nil,
            in: .page
          ) { result in
            switch result {
            case .success(let value):
              self.respond(connection, Self.encode(value))
            case .failure(let error):
              self.respond(connection, "ERR: \(error.localizedDescription)")
            }
          }
        }
        return
      }
      if path == "/eval" {
        let source = body.isEmpty ? "void 0" : body
        DispatchQueue.main.async {
          guard let webView = self.webView else {
            self.respond(connection, "ERR: no webview")
            return
          }
          webView.evaluateJavaScript(source) { result, error in
            if let error {
              self.respond(connection, "ERR: \(error.localizedDescription)")
            } else {
              self.respond(connection, Self.encode(result))
            }
          }
        }
        return
      }
      respond(connection, "ERR: unknown path \(path)", status: "404 Not Found")
    }
  }

  private static func encode(_ value: Any?) -> String {
    switch value {
    case nil:
      return "null"
    case let string as String:
      return string
    case let number as NSNumber:
      return number.stringValue
    default:
      if
        let value,
        JSONSerialization.isValidJSONObject(value),
        let data = try? JSONSerialization.data(withJSONObject: value),
        let string = String(data: data, encoding: .utf8)
      {
        return string
      }
      return String(describing: value ?? "null")
    }
  }

  private func respond(
    _ connection: NWConnection,
    _ body: String,
    status: String = "200 OK"
  ) {
    let payload = Data(body.utf8)
    let header = "HTTP/1.1 \(status)\r\n"
      + "Content-Type: text/plain; charset=utf-8\r\n"
      + "Content-Length: \(payload.count)\r\n"
      + "Connection: close\r\n\r\n"
    connection.send(
      content: Data(header.utf8) + payload,
      completion: .contentProcessed { _ in connection.cancel() }
    )
  }
}
#endif
