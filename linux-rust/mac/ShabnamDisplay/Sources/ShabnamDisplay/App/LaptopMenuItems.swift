import AppKit

/// A slider that lives in the menu: a symbol, the slider, and the value in figures.
final class MenuSlider: NSView {
    let slider: NSSlider
    private let value = NSTextField(labelWithString: "")
    private let suffix: String

    /// - title: what the slider sets, for the tooltip and for VoiceOver.
    init(symbol: String, title: String, suffix: String, target: AnyObject, action: Selector) {
        slider = NSSlider(value: 0, minValue: 0, maxValue: 100, target: target, action: action)
        self.suffix = suffix
        super.init(frame: NSRect(x: 0, y: 0, width: 260, height: 28))
        autoresizingMask = [.width]

        let icon = NSImageView(frame: NSRect(x: 16, y: 5, width: 18, height: 18))
        icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        icon.contentTintColor = .secondaryLabelColor
        addSubview(icon)

        slider.frame = NSRect(x: 40, y: 4, width: 162, height: 20)
        slider.autoresizingMask = [.width]
        slider.isContinuous = true
        slider.controlSize = .small
        slider.toolTip = title
        slider.setAccessibilityLabel(title)
        addSubview(slider)

        value.frame = NSRect(x: 206, y: 6, width: 42, height: 16)
        value.autoresizingMask = [.minXMargin]
        value.alignment = .right
        value.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        value.textColor = .secondaryLabelColor
        addSubview(value)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func show(_ number: Int, of maximum: Int) {
        slider.maxValue = Double(maximum)
        if slider.integerValue != number { slider.integerValue = number }
        showFigure(number)
    }

    func showFigure(_ number: Int) {
        value.stringValue = "\(number)\(suffix)"
    }
}

extension LaptopStatus {
    /// "82% · charging" — nil when the laptop reports no battery.
    var batteryText: String? {
        guard let battery else { return nil }
        if charging { return L("%d%% · charging", battery) }
        if externalPower { return L("%d%% · plugged in", battery) }
        return "\(battery)%"
    }

    /// The menu line: "Shabnam 82% · charging".
    var batteryDescription: String? { batteryText.map { L("Shabnam %@", $0) } }
}
