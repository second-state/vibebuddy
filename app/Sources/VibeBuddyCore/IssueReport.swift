import Foundation

/// "Report a problem": a new GitHub issue with the versions filled in, so the user only has to say what happened.
/// Only build and version lines go in, never logs or anything an agent did.
public enum IssueReport {
    public static let newIssue = "https://github.com/second-state/vibebuddy/issues/new"

    public static func body(summary: String) -> String {
        """
        **What happened?**



        **What did you expect?**



        ---
        ```
        \(summary)
        ```
        """
    }

    public static func url(summary: String) -> URL {
        var components = URLComponents(string: newIssue)!
        components.queryItems = [URLQueryItem(name: "body", value: body(summary: summary))]
        // URLComponents leaves "+" alone, and GitHub would read it as a space.
        components.percentEncodedQuery = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
        return components.url!
    }
}
