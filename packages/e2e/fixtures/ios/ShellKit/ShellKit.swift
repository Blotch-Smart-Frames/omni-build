/// A framework the shell embeds, as a real shell embeds its runtime: code
/// of its own in the app's Frameworks folder, which has its own signature.
/// The app reports its name, which it can only do if the framework loads.
public enum ShellKit {
    public static let name = "ShellKit"
}
