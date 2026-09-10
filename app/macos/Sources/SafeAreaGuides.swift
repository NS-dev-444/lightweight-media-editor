import SwiftUI

/// Where the platform's own interface will cover your video (§19,
/// PRODUCT_DIRECTION.md §7).
///
/// This is the cheapest professional-feeling feature in the product and no
/// lightweight editor has it. Put a caption in the bottom eighth of a Reel and
/// the platform's caption, handle and audio strip sit on top of it — you find
/// out after posting, when it is too late to move.
///
/// The numbers are fractions of the frame, deliberately generous. They are
/// **approximate by nature**: every platform moves its interface between
/// releases and between devices, so a guide that claims pixel accuracy would be
/// lying. What matters is "do not put anything important here", which a
/// slightly-too-large box answers correctly and a slightly-too-small one does
/// not.
struct SafeAreaGuide: Identifiable, Hashable {
    let id: String
    let name: String
    /// Insets as fractions of the frame: top, bottom, leading, trailing.
    let top: CGFloat
    let bottom: CGFloat
    let leading: CGFloat
    let trailing: CGFloat
    /// The shape this guide assumes, for the note shown when it does not match.
    let aspect: CGFloat?

    static let all: [SafeAreaGuide] = [
        SafeAreaGuide(id: "off", name: "Off",
                      top: 0, bottom: 0, leading: 0, trailing: 0, aspect: nil),
        // Reels, TikTok and Shorts all put a caption and handle along the
        // bottom and an action rail down the right.
        SafeAreaGuide(id: "reel", name: "Reels & TikTok",
                      top: 0.10, bottom: 0.22, leading: 0.05, trailing: 0.22,
                      aspect: 9.0 / 16.0),
        SafeAreaGuide(id: "shorts", name: "YouTube Shorts",
                      top: 0.08, bottom: 0.20, leading: 0.05, trailing: 0.18,
                      aspect: 9.0 / 16.0),
        // The classic 90% action-safe / 80% title-safe box.
        SafeAreaGuide(id: "broadcast", name: "Title Safe (90%)",
                      top: 0.05, bottom: 0.05, leading: 0.05, trailing: 0.05,
                      aspect: nil),
    ]

    var isOff: Bool { id == "off" }

    /// The rectangle that stays visible, inside `frame`.
    func safeRect(in frame: CGRect) -> CGRect {
        CGRect(x: frame.minX + frame.width * leading,
               y: frame.minY + frame.height * top,
               width: frame.width * (1 - leading - trailing),
               height: frame.height * (1 - top - bottom))
    }
}

/// Draws the guide over the video, dimming what the platform will cover.
///
/// Dimming rather than outlining, because the question is "what gets covered",
/// and a dimmed region answers it at a glance where a thin rectangle asks you
/// to work it out.
struct SafeAreaOverlay: View {
    let guide: SafeAreaGuide
    /// The video's aspect ratio, so the guide lands on the picture rather than
    /// on the letterbox around it.
    let contentAspect: CGFloat

    var body: some View {
        GeometryReader { geo in
            let frame = videoRect(in: geo.size)
            let safe = guide.safeRect(in: frame)
            ZStack {
                Path { p in
                    p.addRect(frame)
                    p.addRect(safe)
                }
                .fill(Color.black.opacity(0.42), style: FillStyle(eoFill: true))
                Path { p in p.addRect(safe) }
                    .stroke(Color.white.opacity(0.55), lineWidth: 1)
                if let a = guide.aspect, abs(a - contentAspect) > 0.02 {
                    // Saying this is the difference between a helpful guide and
                    // a misleading one: the boxes are only right for the shape
                    // the platform actually shows.
                    VStack {
                        Text("This guide assumes a vertical 9:16 video")
                            .font(.system(size: 10))
                            .padding(.horizontal, 6).padding(.vertical, 3)
                            .background(.black.opacity(0.6))
                            .cornerRadius(4)
                            .padding(.top, 6)
                        Spacer()
                    }
                }
            }
        }
        .allowsHitTesting(false)
    }

    /// Where the video actually sits inside the pane, after letterboxing.
    private func videoRect(in size: CGSize) -> CGRect {
        guard contentAspect > 0, size.width > 0, size.height > 0 else {
            return CGRect(origin: .zero, size: size)
        }
        let paneAspect = size.width / size.height
        if paneAspect > contentAspect {
            let w = size.height * contentAspect
            return CGRect(x: (size.width - w) / 2, y: 0, width: w, height: size.height)
        }
        let h = size.width / contentAspect
        return CGRect(x: 0, y: (size.height - h) / 2, width: size.width, height: h)
    }
}
