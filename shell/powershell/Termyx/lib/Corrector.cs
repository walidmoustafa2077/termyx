using System;
using System.Collections.Generic;
using System.IO;

namespace Termyx
{
    public static class Corrector
    {
        private static readonly object Sync = new object();
        private const int MaxDistanceTwoLength = 6;

        private static HashSet<string> _words = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<int, string[]> _wordsByLength = new Dictionary<int, string[]>();
        private static Dictionary<string, int> _frequency = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _wordCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _commandCanonical = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _commandCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, Dictionary<string, string>> _parameters = new Dictionary<string, Dictionary<string, string>>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _parameterCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

        public static void Initialize(string wordListPath, string userWordListPath)
        {
            HashSet<string> words = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            LoadFile(words, wordListPath);
            LoadFile(words, userWordListPath);

            lock (Sync)
            {
                _words = words;
                _wordsByLength = BuildLengthIndex(words);
                _frequency = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
                _wordCache.Clear();
            }
        }

        public static void LoadFrequencyFile(string path)
        {
            Dictionary<string, int> ranks = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);

            if (!string.IsNullOrEmpty(path) && File.Exists(path))
            {
                int rank = 0;
                using (StreamReader reader = new StreamReader(path))
                {
                    string line;
                    while ((line = reader.ReadLine()) != null)
                    {
                        line = line.Trim();
                        if (line.Length == 0 || line[0] == '#') continue;

                        int space = line.IndexOf(' ');
                        string word = space < 0 ? line : line.Substring(0, space);
                        if (word.Length == 0 || ranks.ContainsKey(word)) continue;

                        ranks[word] = rank;
                        rank++;
                    }
                }
            }

            lock (Sync)
            {
                _frequency = ranks;
                _wordCache.Clear();
            }
        }

        public static int FrequencyCount
        {
            get { lock (Sync) { return _frequency.Count; } }
        }

        public static bool IsKnownWord(string word)
        {
            if (string.IsNullOrEmpty(word)) return false;
            lock (Sync) { return _words.Contains(word); }
        }

        public static bool IsKnownCommand(string command)
        {
            if (string.IsNullOrEmpty(command)) return false;
            lock (Sync) { return _commandCanonical.ContainsKey(command); }
        }

        public static string SuggestWord(string word, int maxDistance)
        {
            if (string.IsNullOrEmpty(word) || word.Length < 2) return null;

            lock (Sync)
            {
                if (_words.Contains(word)) return word;
            }

            string key = maxDistance.ToString() + "|" + word.ToLowerInvariant();
            lock (Sync)
            {
                string cached;
                if (_wordCache.TryGetValue(key, out cached))
                {
                    if (cached.Length == 0) return null;
                    return MatchCase(word, cached);
                }
            }

            Dictionary<int, string[]> index;
            lock (Sync) { index = _wordsByLength; }

            string found = SuggestFromIndex(index, word.ToLowerInvariant(), maxDistance, MaxDistanceTwoLength, true);
            lock (Sync) { _wordCache[key] = found == null ? string.Empty : found; }

            if (found == null) return null;
            return MatchCase(word, found);
        }

        public static void AddUserWord(string word)
        {
            if (string.IsNullOrWhiteSpace(word)) return;

            string normalized = word.Trim().ToLowerInvariant();
            lock (Sync)
            {
                _words.Add(normalized);
                _wordsByLength = BuildLengthIndex(_words);
                _wordCache.Clear();
            }
        }

        public static void SetCommands(string[] commands)
        {
            SetCommands((IEnumerable<string>)commands);
        }

        public static void SetCommands(IEnumerable<string> commands)
        {
            Dictionary<string, string> map = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            if (commands != null)
            {
                foreach (string name in commands)
                {
                    if (string.IsNullOrWhiteSpace(name)) continue;
                    string trimmed = name.Trim();
                    if (!map.ContainsKey(trimmed)) map[trimmed] = trimmed;
                }
            }

            lock (Sync)
            {
                _commandCanonical = map;
                _commandCache.Clear();
            }
        }

        public static string SuggestCommand(string word, int maxDistance)
        {
            if (string.IsNullOrEmpty(word)) return null;

            string key = maxDistance.ToString() + "|" + word.ToLowerInvariant();
            Dictionary<string, string> map;

            lock (Sync)
            {
                string cached;
                if (_commandCache.TryGetValue(key, out cached))
                {
                    return cached.Length == 0 ? null : cached;
                }
                map = _commandCanonical;
            }

            string canonical;
            if (map.TryGetValue(word, out canonical))
            {
                lock (Sync) { _commandCache[key] = canonical; }
                return canonical;
            }

            string found = SuggestCanonical(map, word.ToLowerInvariant(), maxDistance);
            lock (Sync) { _commandCache[key] = found == null ? string.Empty : found; }
            return found;
        }

        public static void SetParameters(string commandName, string[] names)
        {
            SetParameters(commandName, (IEnumerable<string>)names);
        }

        public static void SetParameters(string commandName, IEnumerable<string> names)
        {
            if (string.IsNullOrEmpty(commandName)) return;

            Dictionary<string, string> map = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            if (names != null)
            {
                foreach (string name in names)
                {
                    if (string.IsNullOrWhiteSpace(name)) continue;
                    string trimmed = name.Trim();
                    if (!map.ContainsKey(trimmed)) map[trimmed] = trimmed;
                }
            }

            lock (Sync)
            {
                _parameters[commandName] = map;
                _parameterCache.Clear();
            }
        }

        public static string SuggestParameter(string command, string word, int maxDistance)
        {
            if (string.IsNullOrEmpty(command) || string.IsNullOrEmpty(word)) return null;

            string key = command.ToLowerInvariant() + "|" + maxDistance.ToString() + "|" + word.ToLowerInvariant();
            Dictionary<string, string> map;

            lock (Sync)
            {
                string cached;
                if (_parameterCache.TryGetValue(key, out cached))
                {
                    return cached.Length == 0 ? null : cached;
                }
                if (!_parameters.TryGetValue(command, out map)) return null;
            }

            string found = SuggestCanonical(map, word.ToLowerInvariant(), maxDistance);
            lock (Sync) { _parameterCache[key] = found == null ? string.Empty : found; }
            return found;
        }

        public static int WordCount
        {
            get { lock (Sync) { return _words.Count; } }
        }

        public static int CommandCount
        {
            get { lock (Sync) { return _commandCanonical.Count; } }
        }

        private static void LoadFile(HashSet<string> target, string path)
        {
            if (string.IsNullOrEmpty(path) || !File.Exists(path)) return;

            using (StreamReader reader = new StreamReader(path))
            {
                string line;
                while ((line = reader.ReadLine()) != null)
                {
                    line = line.Trim();
                    if (line.Length == 0 || line[0] == '#') continue;
                    target.Add(line);
                }
            }
        }

        private static Dictionary<int, string[]> BuildLengthIndex(HashSet<string> words)
        {
            Dictionary<int, List<string>> buckets = new Dictionary<int, List<string>>();
            foreach (string word in words)
            {
                if (string.IsNullOrEmpty(word)) continue;

                List<string> bucket;
                if (!buckets.TryGetValue(word.Length, out bucket))
                {
                    bucket = new List<string>();
                    buckets[word.Length] = bucket;
                }
                bucket.Add(word);
            }

            Dictionary<int, string[]> index = new Dictionary<int, string[]>(buckets.Count);
            foreach (KeyValuePair<int, List<string>> pair in buckets)
            {
                index[pair.Key] = pair.Value.ToArray();
            }
            return index;
        }

        private static string SuggestFromIndex(Dictionary<int, string[]> index, string lowerWord, int maxDistance, int distanceTwoMaxLength, bool breakTiesByFrequency)
        {
            if (index.Count == 0) return null;

            int maxLength = 0;
            foreach (int length in index.Keys)
            {
                if (length > maxLength) maxLength = length;
            }

            int[] row0 = new int[maxLength + 1];
            int[] row1 = new int[maxLength + 1];
            int[] row2 = new int[maxLength + 1];

            int wordLength = lowerWord.Length;

            List<string> oneEdit = breakTiesByFrequency ? new List<string>() : null;
            int countOne = 0;
            string foundOne = null;
            for (int length = wordLength - 1; length <= wordLength + 1; length++)
            {
                string[] bucket;
                if (!index.TryGetValue(length, out bucket)) continue;

                foreach (string candidate in bucket)
                {
                    if (BoundedDistance(lowerWord, candidate, 1, row0, row1, row2) != 1) continue;

                    countOne++;
                    foundOne = candidate;
                    if (breakTiesByFrequency)
                    {
                        oneEdit.Add(candidate);
                    }
                    else if (countOne > 1)
                    {
                        return null;
                    }
                }
            }

            if (countOne == 1) return foundOne.ToLowerInvariant();
            if (countOne > 1)
            {
                string bestOne = PickMostFrequent(oneEdit, wordLength);
                return bestOne == null ? null : bestOne.ToLowerInvariant();
            }

            if (maxDistance < 2 || wordLength > distanceTwoMaxLength) return null;

            List<string> twoEdit = breakTiesByFrequency ? new List<string>() : null;
            int countTwo = 0;
            string foundTwo = null;
            for (int length = wordLength - 2; length <= wordLength + 2; length++)
            {
                string[] bucket;
                if (!index.TryGetValue(length, out bucket)) continue;

                foreach (string candidate in bucket)
                {
                    if (BoundedDistance(lowerWord, candidate, 2, row0, row1, row2) != 2) continue;

                    countTwo++;
                    foundTwo = candidate;
                    if (breakTiesByFrequency)
                    {
                        twoEdit.Add(candidate);
                    }
                    else if (countTwo > 1)
                    {
                        return null;
                    }
                }
            }

            if (countTwo == 1) return foundTwo.ToLowerInvariant();
            if (countTwo > 1)
            {
                string bestTwo = PickMostFrequent(twoEdit, wordLength);
                return bestTwo == null ? null : bestTwo.ToLowerInvariant();
            }
            return null;
        }

        private static string PickMostFrequent(List<string> candidates, int tokenLength)
        {
            if (candidates == null || candidates.Count == 0) return null;

            // Deleting a letter is the most destructive correction, so a candidate that is
            // at least as long as the typed word wins before any shorter one is considered.
            List<string> preferred = new List<string>();
            foreach (string candidate in candidates)
            {
                if (candidate.Length >= tokenLength) preferred.Add(candidate);
            }
            if (preferred.Count == 0) preferred = candidates;

            Dictionary<string, int> ranks;
            lock (Sync) { ranks = _frequency; }
            if (ranks.Count == 0) return null;

            string best = null;
            int bestRank = int.MaxValue;
            foreach (string candidate in preferred)
            {
                int rank;
                if (!ranks.TryGetValue(candidate, out rank)) continue;
                if (rank < bestRank)
                {
                    bestRank = rank;
                    best = candidate;
                }
            }
            return best;
        }

        private static int BoundedDistance(string a, string b, int max, int[] row0, int[] row1, int[] row2)
        {
            int n = a.Length;
            int m = b.Length;

            if (Math.Abs(n - m) > max) return -1;

            for (int j = 0; j <= m; j++) row0[j] = j;

            int[] previous = row0;
            int[] current = row1;
            int[] previousPrevious = row2;

            for (int i = 1; i <= n; i++)
            {
                current[0] = i;
                int rowMinimum = current[0];

                for (int j = 1; j <= m; j++)
                {
                    int cost = (a[i - 1] == b[j - 1]) ? 0 : 1;
                    int value = Math.Min(Math.Min(previous[j] + 1, current[j - 1] + 1), previous[j - 1] + cost);

                    if (i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1])
                    {
                        value = Math.Min(value, previousPrevious[j - 2] + 1);
                    }

                    current[j] = value;
                    if (value < rowMinimum) rowMinimum = value;
                }

                if (rowMinimum > max) return -1;

                int[] spare = previousPrevious;
                previousPrevious = previous;
                previous = current;
                current = spare;
            }

            return previous[m] <= max ? previous[m] : -1;
        }

        private static string SuggestCanonical(Dictionary<string, string> map, string lowerWord, int maxDistance)
        {
            if (map.Count == 0) return null;

            HashSet<string> names = new HashSet<string>(map.Keys, StringComparer.OrdinalIgnoreCase);
            HashSet<string> found = CollectEdits1(names, lowerWord);

            if (found.Count == 1)
            {
                foreach (string only in found) return map[only];
            }
            if (found.Count > 1) return null;
            if (maxDistance < 2) return null;

            string best = null;
            int bestDistance = int.MaxValue;
            int bestCount = 0;

            foreach (string name in map.Keys)
            {
                int distance = Distance(lowerWord, name.ToLowerInvariant(), maxDistance);
                if (distance < 0) continue;

                if (distance < bestDistance)
                {
                    bestDistance = distance;
                    best = name;
                    bestCount = 1;
                }
                else if (distance == bestDistance)
                {
                    bestCount++;
                }
            }

            if (bestCount != 1) return null;
            return map[best];
        }

        private static int Distance(string a, string b, int max)
        {
            if (Math.Abs(a.Length - b.Length) > max) return -1;

            int n = a.Length;
            int m = b.Length;
            int[,] d = new int[n + 1, m + 1];

            for (int i = 0; i <= n; i++) d[i, 0] = i;
            for (int j = 0; j <= m; j++) d[0, j] = j;

            for (int i = 1; i <= n; i++)
            {
                for (int j = 1; j <= m; j++)
                {
                    int cost = (a[i - 1] == b[j - 1]) ? 0 : 1;
                    int value = Math.Min(Math.Min(d[i - 1, j] + 1, d[i, j - 1] + 1), d[i - 1, j - 1] + cost);

                    if (i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1])
                    {
                        value = Math.Min(value, d[i - 2, j - 2] + 1);
                    }

                    d[i, j] = value;
                }
            }

            return d[n, m] > max ? -1 : d[n, m];
        }

        private static HashSet<string> CollectEdits1(HashSet<string> set, string lowerWord)
        {
            HashSet<string> found = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (string candidate in Edits1(lowerWord))
            {
                if (set.Contains(candidate)) found.Add(candidate);
            }
            return found;
        }

        private static IEnumerable<string> Edits1(string word)
        {
            HashSet<string> result = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            int length = word.Length;

            for (int i = 0; i < length; i++)
            {
                string left = word.Substring(0, i);
                string right = word.Substring(i);

                result.Add(left + right.Substring(1));

                if (i < length - 1)
                {
                    result.Add(left + word[i + 1] + word[i] + word.Substring(i + 2));
                }

                for (char c = 'a'; c <= 'z'; c++)
                {
                    if (c != word[i])
                    {
                        result.Add(left + c + right.Substring(1));
                    }
                    result.Add(left + c + right);
                }
            }

            for (char c = 'a'; c <= 'z'; c++)
            {
                result.Add(word + c);
            }

            return result;
        }

        private static string MatchCase(string input, string candidate)
        {
            bool hasUpper = false;
            bool hasLower = false;
            for (int i = 0; i < input.Length; i++)
            {
                if (char.IsUpper(input[i])) hasUpper = true;
                else if (char.IsLower(input[i])) hasLower = true;
            }

            if (hasUpper && !hasLower)
            {
                return candidate.ToUpperInvariant();
            }
            if (char.IsUpper(input[0]))
            {
                return char.ToUpperInvariant(candidate[0]) + candidate.Substring(1);
            }
            return candidate.ToLowerInvariant();
        }
    }
}
