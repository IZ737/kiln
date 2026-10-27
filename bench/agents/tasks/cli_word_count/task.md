Write a command-line program in this folder.

`kiln run . -- FILE` prints the three most common words of FILE, one per
line as `word count`, most common first; words with the same count are in
alphabetical order. A word is a run of ASCII letters, compared lower-cased;
everything else separates words. With no FILE, or a file that cannot be
read, it prints a short message on standard error and exits with code 1.

Put the counting in a folder module `words`, as

    pub struct Count { word: String, count: Int }
    pub fn top(text: String, n: Int) -> List<Count>

which returns at most `n` counts. Add unit tests for it.
