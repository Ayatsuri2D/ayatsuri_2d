// Ayatsuri2D for Unity. Apache-2.0.

using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Ayatsuri2D.Editor
{
    /// <summary>
    /// Just enough JSON for model3.json: objects become dictionaries,
    /// arrays lists, numbers doubles. JsonUtility cannot read the motion
    /// groups, whose names are keys.
    /// </summary>
    internal static class Json
    {
        public static object Parse(string text)
        {
            int i = 0;
            object v = Value(text, ref i);
            Space(text, ref i);
            if (i != text.Length) throw new System.FormatException($"unexpected text at {i}");
            return v;
        }

        public static Dictionary<string, object> Obj(object o, string key) =>
            o is Dictionary<string, object> d && d.TryGetValue(key, out object v) ? v as Dictionary<string, object> : null;

        public static List<object> Arr(object o, string key) =>
            o is Dictionary<string, object> d && d.TryGetValue(key, out object v) ? v as List<object> : null;

        public static string Str(object o, string key) =>
            o is Dictionary<string, object> d && d.TryGetValue(key, out object v) ? v as string : null;

        static object Value(string s, ref int i)
        {
            Space(s, ref i);
            if (i >= s.Length) throw new System.FormatException("unexpected end");
            char c = s[i];
            if (c == '{') return Object(s, ref i);
            if (c == '[') return Array(s, ref i);
            if (c == '"') return String(s, ref i);
            if (Word(s, ref i, "true")) return true;
            if (Word(s, ref i, "false")) return false;
            if (Word(s, ref i, "null")) return null;
            return Number(s, ref i);
        }

        static Dictionary<string, object> Object(string s, ref int i)
        {
            var d = new Dictionary<string, object>();
            i++;
            Space(s, ref i);
            if (Take(s, ref i, '}')) return d;
            do
            {
                Space(s, ref i);
                string k = String(s, ref i);
                Space(s, ref i);
                Expect(s, ref i, ':');
                d[k] = Value(s, ref i);
                Space(s, ref i);
            } while (Take(s, ref i, ','));
            Expect(s, ref i, '}');
            return d;
        }

        static List<object> Array(string s, ref int i)
        {
            var l = new List<object>();
            i++;
            Space(s, ref i);
            if (Take(s, ref i, ']')) return l;
            do
            {
                l.Add(Value(s, ref i));
                Space(s, ref i);
            } while (Take(s, ref i, ','));
            Expect(s, ref i, ']');
            return l;
        }

        static string String(string s, ref int i)
        {
            Expect(s, ref i, '"');
            var b = new StringBuilder();
            while (i < s.Length && s[i] != '"')
            {
                char c = s[i++];
                if (c != '\\')
                {
                    b.Append(c);
                    continue;
                }
                if (i >= s.Length) break;
                char e = s[i++];
                switch (e)
                {
                    case 'n': b.Append('\n'); break;
                    case 't': b.Append('\t'); break;
                    case 'r': b.Append('\r'); break;
                    case 'b': b.Append('\b'); break;
                    case 'f': b.Append('\f'); break;
                    case 'u':
                        if (i + 4 > s.Length) throw new System.FormatException("short \\u escape");
                        b.Append((char)int.Parse(s.Substring(i, 4), NumberStyles.HexNumber));
                        i += 4;
                        break;
                    default: b.Append(e); break;
                }
            }
            Expect(s, ref i, '"');
            return b.ToString();
        }

        static double Number(string s, ref int i)
        {
            int start = i;
            while (i < s.Length && "+-0123456789.eE".IndexOf(s[i]) >= 0) i++;
            if (i == start) throw new System.FormatException($"unexpected '{s[i]}' at {i}");
            return double.Parse(s.Substring(start, i - start), NumberStyles.Float, CultureInfo.InvariantCulture);
        }

        static bool Word(string s, ref int i, string w)
        {
            if (string.CompareOrdinal(s, i, w, 0, w.Length) != 0) return false;
            i += w.Length;
            return true;
        }

        static bool Take(string s, ref int i, char c)
        {
            if (i < s.Length && s[i] == c)
            {
                i++;
                return true;
            }
            return false;
        }

        static void Expect(string s, ref int i, char c)
        {
            if (!Take(s, ref i, c)) throw new System.FormatException($"expected '{c}' at {i}");
        }

        static void Space(string s, ref int i)
        {
            while (i < s.Length && char.IsWhiteSpace(s[i])) i++;
            // a UTF-8 byte order mark read as text
            while (i < s.Length && s[i] == '﻿') i++;
        }
    }
}
