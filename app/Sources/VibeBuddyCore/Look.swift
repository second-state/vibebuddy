import Foundation

/// An image as plain pixels: width × height RGBA, 8 bits each, row by row. The app decodes files into
/// this so the look building below stays free of AppKit and can be self-tested.
public struct RGBAImage: Equatable, Sendable {
    public var width: Int
    public var height: Int
    public var pixels: [UInt8]

    public init(width: Int, height: Int, pixels: [UInt8]) {
        precondition(pixels.count == width * height * 4)
        self.width = width; self.height = height; self.pixels = pixels
    }

    public func pixel(_ x: Int, _ y: Int) -> (UInt8, UInt8, UInt8, UInt8) {
        let at = (y * width + x) * 4
        return (pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3])
    }
}

/// Turns a user's drawings into a look (firmware-rs/core/src/look.rs), the way tools/draft-look.py and
/// tools/make-look.py do for the preset Characters: the background goes, each figure is cropped and
/// scaled by the same factor, set bottom-center in a 48 × 64 cell, and the four frames share 15 colors
/// plus transparent. The app never draws: the images come from whatever tool the user likes (ADR-0009).
public enum LookBuilder {
    public static let width = 48
    public static let height = 64
    static let figureHeight = 60
    static let colors = 16

    public enum Failure: Error, Equatable {
        case empty
        case tooManyImages
    }

    /// One image per frame, in the order normal, eyes closed, happy, sad. Fewer than four reuse the
    /// normal one for the frames that are missing.
    public static func build(_ images: [RGBAImage]) throws -> Data {
        guard !images.isEmpty else { throw Failure.empty }
        guard images.count <= 4 else { throw Failure.tooManyImages }
        let figures = images.map { cutOut($0) }
        let frames = (0..<4).map { figures[$0 < figures.count ? $0 : 0] }
        guard let normalBox = boundingBox(frames[0]) else { throw Failure.empty }
        let scale = Double(figureHeight) / Double(normalBox.maxY - normalBox.minY)
        let cells = try frames.map { figure -> [(UInt8, UInt8, UInt8)?] in
            guard let box = boundingBox(figure) else { throw Failure.empty }
            return cell(figure, box: box, scale: scale)
        }
        return encode(cells)
    }

    /// Opaque means alpha of at least 128 in an image that has transparency. An image without any
    /// (a white or magenta background) has its background flood-filled away from the edges, so white
    /// inside the figure stays.
    static func opaqueMask(_ image: RGBAImage) -> [Bool] {
        let count = image.width * image.height
        var opaque = [Bool](repeating: true, count: count)
        var hasAlpha = false
        for index in 0..<count where image.pixels[index * 4 + 3] < 128 {
            opaque[index] = false
            hasAlpha = true
        }
        if hasAlpha { return opaque }
        func background(_ index: Int) -> Bool {
            let (r, g, b) = (image.pixels[index * 4], image.pixels[index * 4 + 1], image.pixels[index * 4 + 2])
            return min(r, g, b) > 225 || (r > 230 && g < 30 && b > 230)
        }
        var stack: [Int] = []
        for x in 0..<image.width { stack += [x, (image.height - 1) * image.width + x] }
        for y in 0..<image.height { stack += [y * image.width, y * image.width + image.width - 1] }
        while let index = stack.popLast() {
            guard opaque[index], background(index) else { continue }
            opaque[index] = false
            let x = index % image.width, y = index / image.width
            if x > 0 { stack.append(index - 1) }
            if x + 1 < image.width { stack.append(index + 1) }
            if y > 0 { stack.append(index - image.width) }
            if y + 1 < image.height { stack.append(index + image.width) }
        }
        return opaque
    }

    struct Figure {
        var image: RGBAImage
        var opaque: [Bool]
    }

    static func cutOut(_ image: RGBAImage) -> Figure {
        Figure(image: image, opaque: opaqueMask(image))
    }

    struct Box { var minX, minY, maxX, maxY: Int }

    static func boundingBox(_ figure: Figure) -> Box? {
        var box: Box?
        for y in 0..<figure.image.height {
            for x in 0..<figure.image.width where figure.opaque[y * figure.image.width + x] {
                if box == nil { box = Box(minX: x, minY: y, maxX: x + 1, maxY: y + 1) }
                box!.minX = min(box!.minX, x); box!.maxX = max(box!.maxX, x + 1)
                box!.minY = min(box!.minY, y); box!.maxY = max(box!.maxY, y + 1)
            }
        }
        return box
    }

    /// Shrinks the figure into a cell: each cell pixel averages the opaque source pixels under it, and
    /// is kept only where the figure covers most of it, so there is no halo.
    static func cell(_ figure: Figure, box: Box, scale: Double) -> [(UInt8, UInt8, UInt8)?] {
        let sourceW = box.maxX - box.minX, sourceH = box.maxY - box.minY
        let w = min(width, max(1, Int((Double(sourceW) * scale).rounded())))
        let h = min(height, max(1, Int((Double(sourceH) * scale).rounded())))
        var out = [(UInt8, UInt8, UInt8)?](repeating: nil, count: width * height)
        let left = (width - w) / 2, top = height - h
        for cy in 0..<h {
            for cx in 0..<w {
                let x0 = box.minX + cx * sourceW / w, x1 = max(x0 + 1, box.minX + (cx + 1) * sourceW / w)
                let y0 = box.minY + cy * sourceH / h, y1 = max(y0 + 1, box.minY + (cy + 1) * sourceH / h)
                var sum = (0, 0, 0), covered = 0
                for y in y0..<y1 {
                    for x in x0..<x1 where figure.opaque[y * figure.image.width + x] {
                        let (r, g, b, _) = figure.image.pixel(x, y)
                        sum.0 += Int(r); sum.1 += Int(g); sum.2 += Int(b); covered += 1
                    }
                }
                guard covered * 100 >= (x1 - x0) * (y1 - y0) * 55 else { continue }
                out[(top + cy) * width + left + cx] = (UInt8(sum.0 / covered), UInt8(sum.1 / covered), UInt8(sum.2 / covered))
            }
        }
        return out
    }

    /// Median cut down to 15 colors, shared by the four frames; index 0 is transparent.
    static func palette(_ colors: [(UInt8, UInt8, UInt8)], size: Int) -> [(UInt8, UInt8, UInt8)] {
        var boxes = [colors]
        while boxes.count < size {
            guard let (index, channel) = boxes.enumerated().compactMap({ index, box -> (Int, Int, Int)? in
                guard box.count > 1 else { return nil }
                let ranges = (0..<3).map { c in
                    let values = box.map { [Int($0.0), Int($0.1), Int($0.2)][c] }
                    return values.max()! - values.min()!
                }
                let channel = ranges.firstIndex(of: ranges.max()!)!
                return (index, channel, ranges[channel])
            }).max(by: { $0.2 < $1.2 }).map({ ($0.0, $0.1) }) else { break }
            let sorted = boxes[index].sorted { [Int($0.0), Int($0.1), Int($0.2)][channel] < [Int($1.0), Int($1.1), Int($1.2)][channel] }
            boxes[index] = Array(sorted[..<(sorted.count / 2)])
            boxes.append(Array(sorted[(sorted.count / 2)...]))
        }
        return boxes.filter { !$0.isEmpty }.map { box in
            let n = box.count
            return (UInt8(box.map { Int($0.0) }.reduce(0, +) / n), UInt8(box.map { Int($0.1) }.reduce(0, +) / n), UInt8(box.map { Int($0.2) }.reduce(0, +) / n))
        }
    }

    static func rgb565(_ c: (UInt8, UInt8, UInt8)) -> UInt16 {
        UInt16(c.0 >> 3) << 11 | UInt16(c.1 >> 2) << 5 | UInt16(c.2 >> 3)
    }

    static func encode(_ cells: [[(UInt8, UInt8, UInt8)?]]) -> Data {
        let opaque = cells.flatMap { $0.compactMap { $0 } }
        let colors = palette(opaque, size: colors - 1)
        func nearest(_ c: (UInt8, UInt8, UInt8)) -> UInt8 {
            func distance(_ p: (UInt8, UInt8, UInt8)) -> Int {
                let d = (Int(c.0) - Int(p.0), Int(c.1) - Int(p.1), Int(c.2) - Int(p.2))
                return d.0 * d.0 + d.1 * d.1 + d.2 * d.2
            }
            return UInt8(colors.indices.min { distance(colors[$0]) < distance(colors[$1]) }! + 1)
        }
        var out = Data("LOOK".utf8) + Data([UInt8(width), UInt8(height), 4, 0])
        let entries = [UInt16(0)] + colors.map(rgb565) + Array(repeating: 0, count: Self.colors - 1 - colors.count)
        for entry in entries { out += Data([UInt8(entry & 0xFF), UInt8(entry >> 8)]) }
        for cell in cells {
            let indices = cell.map { $0.map(nearest) ?? 0 }
            for pair in stride(from: 0, to: indices.count, by: 2) {
                out.append(indices[pair] | indices[pair + 1] << 4)
            }
        }
        return out
    }
}

extension LookBuilder {
    /// The four frames of a look as images, for showing it in the app; nil if it isn't a look.
    public static func frames(of look: Data) -> [RGBAImage]? {
        let look = Data(look)
        guard look.count == 8 + colors * 2 + 4 * width * height / 2, look.prefix(4) == Data("LOOK".utf8) else { return nil }
        // Explicit types and short expressions: Xcode 16's type checker times out on the inferred version.
        let palette: [(UInt8, UInt8, UInt8)] = (0..<colors).map { (index: Int) -> (UInt8, UInt8, UInt8) in
            let entry: Int = Int(look[8 + index * 2]) | Int(look[9 + index * 2]) << 8
            let r = UInt8((entry >> 11) * 255 / 31)
            let g = UInt8(((entry >> 5) & 0x3F) * 255 / 63)
            let b = UInt8((entry & 0x1F) * 255 / 31)
            return (r, g, b)
        }
        let cells = width * height
        let firstFrame = 8 + colors * 2
        return (0..<4).map { (frame: Int) -> RGBAImage in
            var pixels = [UInt8](repeating: 0, count: cells * 4)
            let start: Int = firstFrame + frame * cells / 2
            for at in 0..<cells {
                let byte: UInt8 = look[start + at / 2]
                let index = Int(at % 2 == 0 ? byte & 15 : byte >> 4)
                guard index != 0 else { continue }
                let (r, g, b) = palette[index]
                pixels.replaceSubrange((at * 4)..<(at * 4 + 4), with: [r, g, b, 255])
            }
            return RGBAImage(width: width, height: height, pixels: pixels)
        }
    }
}

extension VoicePack {
    /// A new Character made of this pack's voice and lines and a look the user brought: the pack is
    /// rewritten as format version 2 with the look appended and a new id. Nil for an old voice pack,
    /// which has no pools to borrow, or a pack with more lines than version 2 has room for.
    public func withLook(_ look: Data, id: String) -> Data? {
        guard isCharacter, let idBytes = id.data(using: .ascii), !idBytes.isEmpty, idBytes.count <= 31 else { return nil }
        let header = VoicePack.characterHeaderBytes
        let lines = Int(data[54]) | Int(data[55]) << 8
        guard lines <= (1008 - 128) / 8 else { return nil }
        var head = Data(data.prefix(header))
        // Drop any look the base pack already wears.
        let oldLook = data[4] == 2 ? Self.u32(data, 1008) : 0
        var payload = Data(data[header..<(oldLook > 0 ? oldLook : data.count)])
        payload += Data(count: (4 - payload.count % 4) % 4)
        let lookOffset = header + payload.count
        payload += look
        Self.put(&head, 4, 2)
        Self.put(&head, 8, UInt32(payload.count))
        Self.put(&head, 12, CRC32.of(payload))
        head.replaceSubrange(16..<48, with: idBytes + Data(count: 32 - idBytes.count))
        Self.put(&head, 1008, UInt32(lookOffset))
        Self.put(&head, 1012, UInt32(look.count))
        Self.put(&head, 1020, CRC32.of(head.prefix(1020)))
        return head + payload
    }

    public static func u32(_ data: Data, _ at: Int) -> Int {
        Int(data[at]) | Int(data[at + 1]) << 8 | Int(data[at + 2]) << 16 | Int(data[at + 3]) << 24
    }

    static func put(_ data: inout Data, _ at: Int, _ value: UInt32) {
        for index in 0..<4 { data[at + index] = UInt8((value >> (8 * UInt32(index))) & 0xFF) }
    }
}

/// CRC32 as in zlib, which both pack formats use.
public enum CRC32 {
    static let table: [UInt32] = (0..<256).map { n in
        (0..<8).reduce(UInt32(n)) { c, _ in c & 1 != 0 ? 0xEDB8_8320 ^ (c >> 1) : c >> 1 }
    }

    public static func of(_ data: Data) -> UInt32 {
        ~data.reduce(~UInt32(0)) { c, byte in table[Int((c ^ UInt32(byte)) & 0xFF)] ^ (c >> 8) }
    }
}

/// Putting Character packs together in the app: the lines of a pack, and a pack made of lines.
extension VoicePack {
    /// Each occasion's pool, in table order: each line's ADPCM bytes and sample count. Nil for an
    /// old voice pack.
    public var pools: [[(audio: Data, samples: Int)]]? {
        guard isCharacter else { return nil }
        let occasions = Int(data[53])
        return (0..<occasions).map { occasion in
            let first = Int(data[56 + occasion * 4]) | Int(data[57 + occasion * 4]) << 8
            let count = Int(data[58 + occasion * 4]) | Int(data[59 + occasion * 4]) << 8
            return (first..<(first + count)).map { line in
                let offset = Self.u32(data, 128 + line * 8), samples = Self.u32(data, 132 + line * 8)
                return (data.subdata(in: offset..<(offset + (samples + 1) / 2)), samples)
            }
        }
    }

    /// This Character with a form of address: `variant` (characters/<id>/address/<form>.bin) holds the
    /// lines said with it, which take the place of the first lines of each pool, where the same lines
    /// sit said without one. The look and the id stay. Nil if either isn't a Character pack.
    public func withAddress(_ variant: VoicePack) -> Data? {
        guard var pools, let replacements = variant.pools else { return nil }
        for (occasion, lines) in replacements.enumerated() where occasion < pools.count && lines.count <= pools[occasion].count {
            pools[occasion].replaceSubrange(0..<lines.count, with: lines)
        }
        return Self.build(id: voiceID, pools: pools, look: look)
    }

    /// Lays a Character pack out exactly as tools/character_pack.py does.
    public static func build(id: String, pools: [[(audio: Data, samples: Int)]], look: Data?) -> Data? {
        guard let idBytes = id.data(using: .ascii), (1...31).contains(idBytes.count) else { return nil }
        let header = characterHeaderBytes
        let lineCount = pools.reduce(0) { $0 + $1.count }
        guard lineCount <= (look == nil ? (1020 - 128) / 8 : (1008 - 128) / 8), pools.count <= (128 - 56) / 4 else { return nil }
        var head = Data(count: header)
        var payload = Data()
        var line = 0
        for (occasion, lines) in pools.enumerated() {
            put16(&head, 56 + occasion * 4, lines.isEmpty ? 0 : line)
            put16(&head, 58 + occasion * 4, lines.count)
            for (audio, samples) in lines {
                put(&head, 128 + line * 8, UInt32(header + payload.count))
                put(&head, 132 + line * 8, UInt32(samples))
                payload += audio
                line += 1
            }
        }
        if let look {
            payload += Data(count: (4 - payload.count % 4) % 4)
            put(&head, 1008, UInt32(header + payload.count))
            put(&head, 1012, UInt32(look.count))
            payload += look
        }
        head.replaceSubrange(0..<4, with: Data("VBCP".utf8))
        put(&head, 4, look == nil ? 1 : 2)
        put(&head, 8, UInt32(payload.count))
        put(&head, 12, CRC32.of(payload))
        head.replaceSubrange(16..<(16 + idBytes.count), with: idBytes)
        put(&head, 48, 16000)
        head[52] = 1
        head[53] = UInt8(pools.count)
        put16(&head, 54, lineCount)
        put(&head, 1020, CRC32.of(head.prefix(1020)))
        return head + payload
    }

    static func put16(_ data: inout Data, _ at: Int, _ value: Int) {
        data[at] = UInt8(value & 0xFF)
        data[at + 1] = UInt8(value >> 8)
    }
}
