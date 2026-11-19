import Foundation

/// Looks a UI string up in Localizable.strings. The English text is the key, so the app still
/// reads correctly when run outside its bundle (`swift run`), where no string tables exist.
func L(_ key: String) -> String {
    NSLocalizedString(key, comment: "")
}

func L(_ key: String, _ args: CVarArg...) -> String {
    String(format: NSLocalizedString(key, comment: ""), arguments: args)
}
