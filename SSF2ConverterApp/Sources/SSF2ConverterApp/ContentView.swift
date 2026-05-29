import SwiftUI
import AppKit
import UniformTypeIdentifiers

// MARK: - State

enum ConversionState {
    case idle
    case converting
    case success(character: String, outputPath: String, log: String)
    case failure(message: String, log: String)
}

// State for the optional "Export in FrayTools" step (post-conversion publish
// driven by tools/fraytools-harness/export-in-fraytools.js over CDP).
enum ExportState: Equatable {
    case idle
    case running
    case done(fraPath: String)
    case failure(message: String)
}

// Mirror of the Rust-side `conversion_log.json` written next to the exported
// character. The Rust struct is in src/main.rs (write_conversion_log) — keep
// fields in sync. `unknown` are calls with no commands.jsonc entry at all;
// `ssf2_only` were intentionally commented out as `// [SSF2-only: …]`.
struct ConversionLog: Codable {
    struct Entry: Codable, Identifiable {
        let name: String
        let count: Int
        var id: String { name }
    }
    let character: String
    let unknown: [Entry]
    let ssf2_only: [Entry]

    var isEmpty: Bool { unknown.isEmpty && ssf2_only.isEmpty }

    static func load(from charDir: String) -> ConversionLog? {
        let url = URL(fileURLWithPath: charDir).appendingPathComponent("conversion_log.json")
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(ConversionLog.self, from: data)
    }
}

// MARK: - Main View

struct ContentView: View {
    @State private var conversionState: ConversionState = .idle
    @State private var isDropTargeted = false
    @State private var progress: Double = 0
    @State private var statusLine = "Ready"
    @State private var outputDir: URL = {
        Bundle.main.bundleURL
            .deletingLastPathComponent()
            .appendingPathComponent("characters")
    }()
    @State private var miscSSF: URL? = nil
    @State private var conversionLog: ConversionLog? = nil
    @State private var showLogSheet: Bool = false

    // Optional path to the user's FrayTools executable, persisted across
    // launches. Empty = not set (the export button stays disabled). Stored as
    // whatever the user picks (.app bundle or raw executable); resolved to the
    // inner Mach-O binary at use time via `resolvedFrayToolsExe`.
    @AppStorage("frayToolsPath") private var frayToolsPath: String = ""

    @State private var exportState: ExportState = .idle

    // Path to the ssf2_converter binary — bundled inside Contents/MacOS/
    var converterBin: URL {
        Bundle.main.executableURL!
            .deletingLastPathComponent()
            .appendingPathComponent("ssf2_converter")
    }

    // FrayTools is McLeodGaming proprietary software; we only ever invoke the
    // user's own install. If they picked a .app bundle, resolve to the inner
    // executable (Contents/MacOS/<CFBundleExecutable>).
    var resolvedFrayToolsExe: String? {
        guard !frayToolsPath.isEmpty else { return nil }
        if frayToolsPath.hasSuffix(".app") {
            let appURL = URL(fileURLWithPath: frayToolsPath)
            if let bundle = Bundle(url: appURL), let exe = bundle.executableURL {
                return exe.path
            }
            let name = appURL.deletingPathExtension().lastPathComponent
            return appURL.appendingPathComponent("Contents/MacOS/\(name)").path
        }
        return frayToolsPath
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            mainContent
        }
        .frame(width: 700, height: 560)
        .background(Color(nsColor: .windowBackgroundColor))
        .sheet(isPresented: $showLogSheet) {
            ConversionLogSheet(log: conversionLog) { showLogSheet = false }
        }
        .onReceive(NotificationCenter.default.publisher(for: .convertSsfFile)) { note in
            if let url = note.object as? URL {
                startConversion(url: url)
            }
        }
    }

    // MARK: Header

    var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("SSF2 → Fraymakers")
                    .font(.system(size: 20, weight: .semibold))
                Text("Character Converter")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 6) {
                miscSsfPicker
                outputDirPicker
                frayToolsPicker
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 16)
    }

    var frayToolsPicker: some View {
        HStack(spacing: 8) {
            Text("FrayTools:")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
            if !frayToolsPath.isEmpty {
                Text(URL(fileURLWithPath: frayToolsPath).lastPathComponent)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.green)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 160)
                Button("✕") { frayToolsPath = "" }
                    .buttonStyle(.plain)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            } else {
                Text("not set")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .italic()
            }
            Button(frayToolsPath.isEmpty ? "Set" : "Change") {
                let panel = NSOpenPanel()
                panel.allowedContentTypes = [.application, .unixExecutable]
                panel.allowsMultipleSelection = false
                panel.canChooseDirectories = false
                panel.treatsFilePackagesAsDirectories = false
                panel.directoryURL = URL(fileURLWithPath: "/Applications")
                panel.prompt = "Select FrayTools"
                if panel.runModal() == .OK, let url = panel.url {
                    frayToolsPath = url.path
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
    }

    var miscSsfPicker: some View {
        HStack(spacing: 8) {
            Text("misc.ssf:")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
            if let misc = miscSSF {
                Text(misc.lastPathComponent)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.green)
                    .lineLimit(1)
                Button("✕") { miscSSF = nil }
                    .buttonStyle(.plain)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            } else {
                Text("auto-detect")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .italic()
            }
            Button("Set") {
                let panel = NSOpenPanel()
                panel.allowedContentTypes = [UTType(filenameExtension: "ssf") ?? .data]
                panel.allowsMultipleSelection = false
                panel.prompt = "Select misc.ssf"
                if panel.runModal() == .OK, let url = panel.url {
                    miscSSF = url
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
    }

    var outputDirPicker: some View {
        HStack(spacing: 8) {
            Text("Output:")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
            Text(outputDir.lastPathComponent)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: 160)
            Button("Change") {
                let panel = NSOpenPanel()
                panel.canChooseDirectories = true
                panel.canChooseFiles = false
                panel.canCreateDirectories = true
                panel.prompt = "Select Output"
                if panel.runModal() == .OK, let url = panel.url {
                    outputDir = url
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
    }

    // MARK: Main content

    @ViewBuilder
    var mainContent: some View {
        switch conversionState {
        case .idle:
            dropZone
        case .converting:
            convertingView
        case .success(let char, let path, let log):
            successView(character: char, outputPath: path, log: log)
        case .failure(let msg, let log):
            failureView(message: msg, log: log)
        }
    }

    // MARK: Drop Zone

    var dropZone: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 16)
                .strokeBorder(
                    isDropTargeted
                        ? Color.accentColor
                        : Color.secondary.opacity(0.3),
                    style: StrokeStyle(lineWidth: 2, dash: [8, 4])
                )
                .background(
                    RoundedRectangle(cornerRadius: 16)
                        .fill(isDropTargeted
                              ? Color.accentColor.opacity(0.08)
                              : Color.clear)
                )
                .animation(.easeInOut(duration: 0.15), value: isDropTargeted)

            VStack(spacing: 16) {
                Image(systemName: "arrow.down.doc.fill")
                    .font(.system(size: 48))
                    .foregroundStyle(isDropTargeted ? Color.accentColor : Color.secondary)
                    .animation(.easeInOut(duration: 0.15), value: isDropTargeted)

                Text("Drop an SSF file here")
                    .font(.system(size: 18, weight: .medium))

                Text("or")
                    .foregroundStyle(.secondary)
                    .font(.system(size: 13))

                Button("Choose File…") { pickFile() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
            }
        }
        .padding(32)
        .onDrop(of: [.fileURL], isTargeted: $isDropTargeted) { providers in
            handleDrop(providers: providers)
        }
    }

    // MARK: Converting View

    var convertingView: some View {
        VStack(spacing: 24) {
            Spacer()
            ProgressView(value: progress)
                .progressViewStyle(.linear)
                .frame(maxWidth: 480)
            Text(statusLine)
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
            Spacer()
        }
        .padding(40)
    }

    // MARK: Success View

    func successView(character: String, outputPath: String, log: String) -> some View {
        VStack(spacing: 20) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 52))
                .foregroundStyle(.green)
            Text("Converted \(character)!")
                .font(.system(size: 20, weight: .semibold))
            Text(outputPath)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: 480)

            ScrollView {
                Text(log)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
            }
            .frame(maxWidth: .infinity, maxHeight: 160)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.secondary.opacity(0.2)))

            exportStatusView(outputPath: outputPath)

            HStack(spacing: 12) {
                Button("Show in Finder") {
                    NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: outputPath)
                }
                .buttonStyle(.bordered)
                if conversionLog != nil {
                    Button("Unhandled Calls…") { showLogSheet = true }
                        .buttonStyle(.bordered)
                }
                exportButton(outputPath: outputPath)
                Button("Convert Another") {
                    conversionState = .idle
                    progress = 0
                    conversionLog = nil
                    exportState = .idle
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(32)
    }

    // MARK: Export-in-FrayTools button + status

    @ViewBuilder
    func exportButton(outputPath: String) -> some View {
        switch exportState {
        case .running:
            Button {
            } label: {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Exporting…")
                }
            }
            .buttonStyle(.bordered)
            .disabled(true)
        default:
            Button("Export in FrayTools") {
                runExport(charOutputPath: outputPath)
            }
            .buttonStyle(.bordered)
            .disabled(resolvedFrayToolsExe == nil)
            .help(resolvedFrayToolsExe == nil
                  ? "Set your FrayTools path in the top-right first"
                  : "Open the converted project in FrayTools and publish the .fra package")
        }
    }

    @ViewBuilder
    func exportStatusView(outputPath: String) -> some View {
        switch exportState {
        case .idle:
            EmptyView()
        case .running:
            Text("Publishing in FrayTools… (this opens FrayTools and runs its Content Exporter)")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
        case .done(let fraPath):
            HStack(spacing: 6) {
                Image(systemName: "shippingbox.fill").foregroundStyle(.green)
                Text("Published:")
                    .font(.system(size: 12, weight: .medium))
                Button(URL(fileURLWithPath: fraPath).lastPathComponent) {
                    NSWorkspace.shared.selectFile(fraPath, inFileViewerRootedAtPath:
                        URL(fileURLWithPath: fraPath).deletingLastPathComponent().path)
                }
                .buttonStyle(.link)
                .font(.system(size: 12, design: .monospaced))
            }
        case .failure(let msg):
            Text("Export failed: \(msg)")
                .font(.system(size: 12))
                .foregroundStyle(.red)
                .multilineTextAlignment(.center)
                .lineLimit(3)
        }
    }

    // MARK: Failure View

    func failureView(message: String, log: String) -> some View {
        VStack(spacing: 20) {
            Image(systemName: "xmark.circle.fill")
                .font(.system(size: 52))
                .foregroundStyle(.red)
            Text("Conversion Failed")
                .font(.system(size: 20, weight: .semibold))
            Text(message)
                .font(.system(size: 13))
                .foregroundStyle(.red)
                .multilineTextAlignment(.center)

            ScrollView {
                Text(log.isEmpty ? "(no output)" : log)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
            }
            .frame(maxWidth: .infinity, maxHeight: 160)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.secondary.opacity(0.2)))

            Button("Try Again") {
                conversionState = .idle
                progress = 0
            }
            .buttonStyle(.borderedProminent)
        }
        .padding(32)
    }

    // MARK: Actions

    func pickFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "ssf") ?? .data]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.prompt = "Convert"
        if panel.runModal() == .OK, let url = panel.url {
            startConversion(url: url)
        }
    }

    func handleDrop(providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }
        provider.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
            guard let data = item as? Data,
                  let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
            DispatchQueue.main.async {
                if url.pathExtension.lowercased() == "ssf" {
                    startConversion(url: url)
                }
            }
        }
        return true
    }

    func startConversion(url: URL) {
        // Auto-detect misc.ssf if not manually set
        if miscSSF == nil {
            let siblingMisc = url.deletingLastPathComponent().appendingPathComponent("misc.ssf")
            if FileManager.default.fileExists(atPath: siblingMisc.path) {
                miscSSF = siblingMisc
            }
        }

        conversionState = .converting
        progress = 0.05
        statusLine = "Starting…"

        let bin = converterBin.path
        let input = url.path
        let outDir = outputDir.path
        let miscPath = miscSSF?.path

        DispatchQueue.global(qos: .userInitiated).async {
            // Animate progress while running
            let steps: [(Double, String)] = [
                (0.15, "Reading SSF file…"),
                (0.30, "Parsing SWF…"),
                (0.50, "Extracting animations…"),
                (0.65, "Extracting images…"),
                (0.80, "Generating Fraymakers files…"),
                (0.92, "Writing output…"),
            ]

            // Check binary exists
            guard FileManager.default.fileExists(atPath: bin) else {
                DispatchQueue.main.async {
                    conversionState = .failure(
                        message: "Converter binary not found at:\n\(bin)",
                        log: ""
                    )
                }
                return
            }

            let process = Process()
            process.executableURL = URL(fileURLWithPath: bin)
            var args = [input, "--output", outDir]
            if let misc = miscPath {
                args += ["--misc-ssf", misc]
            }
            process.arguments = args

            let stdoutPipe = Pipe()
            let stderrPipe = Pipe()
            process.standardOutput = stdoutPipe
            process.standardError  = stderrPipe

            // Tick through progress steps while running
            var stepIdx = 0
            let timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { _ in
                if stepIdx < steps.count {
                    let (prog, msg) = steps[stepIdx]
                    stepIdx += 1
                    DispatchQueue.main.async {
                        withAnimation(.easeInOut(duration: 0.4)) { progress = prog }
                        statusLine = msg
                    }
                }
            }
            RunLoop.main.add(timer, forMode: .common)

            do {
                try process.run()
                process.waitUntilExit()
                timer.invalidate()

                let stdoutData = stdoutPipe.fileHandleForReading.readDataToEndOfFile()
                let stderrData = stderrPipe.fileHandleForReading.readDataToEndOfFile()
                let stdout = String(data: stdoutData, encoding: .utf8) ?? ""
                let stderr = String(data: stderrData, encoding: .utf8) ?? ""
                let log = (stderr + stdout).trimmingCharacters(in: .whitespacesAndNewlines)

                let charName = url.deletingPathExtension().lastPathComponent
                let charOutputPath = outputDir.appendingPathComponent(charName).path

                DispatchQueue.main.async {
                    withAnimation(.easeInOut(duration: 0.3)) { progress = 1.0 }
                    if process.terminationStatus == 0 {
                        let parsedLog = ConversionLog.load(from: charOutputPath)
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                            conversionLog = parsedLog
                            conversionState = .success(
                                character: charName,
                                outputPath: charOutputPath,
                                log: log
                            )
                            if let parsed = parsedLog, !parsed.isEmpty {
                                showLogSheet = true
                            }
                        }
                    } else {
                        conversionState = .failure(
                            message: "Converter exited with code \(process.terminationStatus)",
                            log: log
                        )
                    }
                }
            } catch {
                timer.invalidate()
                DispatchQueue.main.async {
                    conversionState = .failure(message: error.localizedDescription, log: "")
                }
            }
        }
    }

    // MARK: Export in FrayTools

    /// Locate the `node` executable. GUI apps launched from Finder don't
    /// inherit the shell PATH, so check common install locations first, then
    /// fall back to a login shell `which`.
    func findNode() -> String? {
        let candidates = ["/opt/homebrew/bin/node", "/usr/local/bin/node", "/usr/bin/node"]
        for c in candidates where FileManager.default.isExecutableFile(atPath: c) { return c }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/zsh")
        p.arguments = ["-lc", "which node"]
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = Pipe()
        do {
            try p.run(); p.waitUntilExit()
            let out = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if let out, !out.isEmpty, FileManager.default.isExecutableFile(atPath: out) { return out }
        } catch {}
        return nil
    }

    /// Locate tools/fraytools-harness/export-in-fraytools.js by walking up
    /// from the app bundle (the app is built/run from inside the repo).
    func findExportScript() -> String? {
        var dir = Bundle.main.bundleURL
        for _ in 0..<10 {
            let candidate = dir.appendingPathComponent("tools/fraytools-harness/export-in-fraytools.js")
            if FileManager.default.fileExists(atPath: candidate.path) { return candidate.path }
            dir = dir.deletingLastPathComponent()
        }
        return nil
    }

    /// Find the .fraytools project file inside a converted character directory.
    func findProjectFile(in charOutputPath: String) -> String? {
        let dir = URL(fileURLWithPath: charOutputPath)
        guard let entries = try? FileManager.default.contentsOfDirectory(
            at: dir, includingPropertiesForKeys: nil) else { return nil }
        return entries.first { $0.pathExtension == "fraytools" }?.path
    }

    /// Drive the user's FrayTools install to publish the converted project,
    /// via tools/fraytools-harness/export-in-fraytools.js over CDP.
    func runExport(charOutputPath: String) {
        guard let exe = resolvedFrayToolsExe else {
            exportState = .failure(message: "FrayTools path not set"); return
        }
        guard let node = findNode() else {
            exportState = .failure(message: "Couldn't find `node`. Install Node.js (e.g. `brew install node`).")
            return
        }
        guard let script = findExportScript() else {
            exportState = .failure(message: "Couldn't find export-in-fraytools.js under tools/fraytools-harness/")
            return
        }
        guard let project = findProjectFile(in: charOutputPath) else {
            exportState = .failure(message: "No .fraytools project found in \(charOutputPath)")
            return
        }

        exportState = .running

        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: node)
            process.arguments = [script,
                                 "--project", project,
                                 "--fraytools", exe]
            // node resolves chrome-remote-interface from the harness dir.
            process.currentDirectoryURL = URL(fileURLWithPath: script).deletingLastPathComponent()

            let outPipe = Pipe(), errPipe = Pipe()
            process.standardOutput = outPipe
            process.standardError  = errPipe

            do {
                try process.run()
                process.waitUntilExit()
                let outData = outPipe.fileHandleForReading.readDataToEndOfFile()
                let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
                let stdout = String(data: outData, encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                let stderr = String(data: errData, encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

                DispatchQueue.main.async {
                    if process.terminationStatus == 0, !stdout.isEmpty {
                        // Harness prints the .fra path on stdout.
                        let fra = stdout.components(separatedBy: "\n").last ?? stdout
                        exportState = .done(fraPath: fra)
                    } else {
                        // Surface the last line of stderr as the reason.
                        let reason = stderr.components(separatedBy: "\n").last ?? "exit \(process.terminationStatus)"
                        exportState = .failure(message: reason.isEmpty ? "exit \(process.terminationStatus)" : reason)
                    }
                }
            } catch {
                DispatchQueue.main.async {
                    exportState = .failure(message: error.localizedDescription)
                }
            }
        }
    }
}

// MARK: - Conversion Log Sheet

// Auto-presented after a successful conversion when the log has any entries.
// "Unknown" calls are real gaps — names with no entry in any commands.jsonc
// section. "SSF2-only" calls are intentionally surfaced as `// [SSF2-only:…]`
// comments in the generated Haxe and need a manual port (no FM equivalent).
struct ConversionLogSheet: View {
    let log: ConversionLog?
    let onDismiss: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Unhandled Calls")
                        .font(.system(size: 16, weight: .semibold))
                    if let log {
                        Text(log.character)
                            .font(.system(size: 12))
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Button("Done", action: onDismiss)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)

            Divider()

            if let log, !log.isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if !log.unknown.isEmpty {
                            section(
                                title: "Unknown calls",
                                subtitle: "No mapping in commands.jsonc — likely needs a replacement, passthrough, or ssf2_only entry.",
                                entries: log.unknown,
                                color: .orange
                            )
                        }
                        if !log.ssf2_only.isEmpty {
                            section(
                                title: "SSF2-only calls",
                                subtitle: "No Fraymakers equivalent — commented out in the generated Haxe. Manual port required.",
                                entries: log.ssf2_only,
                                color: .blue
                            )
                        }
                    }
                    .padding(16)
                }
            } else {
                VStack(spacing: 8) {
                    Image(systemName: "checkmark.seal.fill")
                        .font(.system(size: 32))
                        .foregroundStyle(.green)
                    Text("All calls handled")
                        .font(.system(size: 14, weight: .medium))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(32)
            }
        }
        .frame(width: 520, height: 460)
    }

    @ViewBuilder
    func section(title: String, subtitle: String, entries: [ConversionLog.Entry], color: Color) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Circle().fill(color).frame(width: 8, height: 8)
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                Text("\(entries.count)")
                    .font(.system(size: 11, weight: .medium))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(color.opacity(0.15))
                    .clipShape(Capsule())
            }
            Text(subtitle)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)

            VStack(spacing: 0) {
                ForEach(entries) { entry in
                    HStack {
                        Text(entry.name)
                            .font(.system(size: 12, design: .monospaced))
                        Spacer()
                        Text("\(entry.count)")
                            .font(.system(size: 11, weight: .medium, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    if entry.id != entries.last?.id {
                        Divider()
                    }
                }
            }
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Color.secondary.opacity(0.2)))
        }
    }
}
