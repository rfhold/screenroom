package android.net;

// JVM-only opaque URI holder; Media3's non-null MIME classifier does not inspect it.
public final class Uri {
    public static final Uri EMPTY = new Uri("");
    private final String value;

    private Uri(String value) { this.value = value; }
    public static Uri parse(String value) { return new Uri(value); }
    @Override public String toString() { return value; }
    @Override public boolean equals(Object other) {
        return other instanceof Uri && value.equals(((Uri) other).value);
    }
    @Override public int hashCode() { return value.hashCode(); }
}
