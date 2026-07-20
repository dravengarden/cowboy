// CowboyDevBridge — DEBUG-only headless WebKit inspection for the iOS shell.
//
// The iOS Simulator cannot be inspected headlessly through
// ios_webkit_debug_proxy, so simulator automation talks to this loopback HTTP
// server instead. It evaluates JavaScript in the shell's one WKWebView and is
// deliberately absent from release builds. See tools/cowboysim.sh.

#if DEBUG
import Foundation
import Network
import WebKit

@objc(CowboyDevBridge) public final class CowboyDevBridge: NSObject {
  private static var shared: CowboyDevBridge?
  private weak var webView: WKWebView?
  private var listener: NWListener?
  private let port: UInt16 = 4171

  @objc public static func installOnWebView(_ webView: WKWebView) {
    if shared == nil { shared = CowboyDevBridge() }
    shared?.webView = webView
    shared?.startIfNeeded()
  }

  private func startIfNeeded() {
    guard listener == nil else { return }
    do {
      let params = NWParameters.tcp
      params.allowLocalEndpointReuse = true
      let listener = try NWListener(
        using: params,
        on: NWEndpoint.Port(rawValue: port)!
      )
      listener.newConnectionHandler = { [weak self] connection in
        self?.handle(connection)
      }
      listener.start(queue: .global(qos: .utility))
      self.listener = listener
      NSLog("[CowboyDevBridge] eval server on 127.0.0.1:\(port) (DEBUG)")
    } catch {
      NSLog("[CowboyDevBridge] failed to start: \(error)")
    }
  }

  private func handle(_ connection: NWConnection) {
    connection.start(queue: .global(qos: .utility))
    connection.receive(minimumIncompleteLength: 1, maximumLength: 1 << 20) {
      [weak self] data, _, _, _ in
      guard
        let self,
        let data,
        let request = String(data: data, encoding: .utf8)
      else {
        connection.cancel()
        return
      }

      let requestLine = request.components(separatedBy: "\r\n").first ?? ""
      let parts = requestLine.split(separator: " ")
      let path = parts.count > 1 ? String(parts[1]) : "/"
      let body = request.range(of: "\r\n\r\n")
        .map { String(request[$0.upperBound...]) } ?? ""

      if path.hasPrefix("/ping") {
        respond(connection, "ok")
        return
      }
      if path.hasPrefix("/aeval") {
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
      if path.hasPrefix("/eval") {
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
      + "Access-Control-Allow-Origin: *\r\n"
      + "Content-Length: \(payload.count)\r\n"
      + "Connection: close\r\n\r\n"
    connection.send(
      content: Data(header.utf8) + payload,
      completion: .contentProcessed { _ in connection.cancel() }
    )
  }
}
#endif
