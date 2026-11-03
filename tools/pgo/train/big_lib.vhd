-- pgo: top=big_lib_tb std=2008 stop=2us analyze=6
-- Reproduces the analysis and elaboration cost of a large utility library in one file: many
-- packages and bodies (strings, conversions, enumerations, records with operators, protected
-- types, access types, generic packages and instances, contexts, aliases, attributes), then a
-- short testbench that calls a sample of it.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Common types and constants.
package bl_types is
  subtype nat8 is natural range 0 to 255;
  subtype nat16 is natural range 0 to 65535;
  subtype small_int is integer range -128 to 127;
  subtype pos_small is positive range 1 to 64;
  subtype byte_slv is std_logic_vector(7 downto 0);
  subtype word_slv is std_logic_vector(31 downto 0);
  type int_vec is array (natural range <>) of integer;
  type real_vec is array (natural range <>) of real;
  type bool_vec is array (natural range <>) of boolean;
  type time_vec is array (natural range <>) of time;
  type slv_vec is array (natural range <>) of std_logic_vector;
  type string_vec is array (natural range <>) of string;
  type byte_array is array (natural range <>) of byte_slv;
  type matrix_t is array (natural range <>, natural range <>) of integer;
  type sev_count_t is array (severity_level) of natural;
  type point_t is record
    x : integer;
    y : integer;
  end record;
  type str_ptr is access string;
  type int_ptr is access integer;
  type char_class_t is (cc_control, cc_space, cc_digit, cc_upper, cc_lower, cc_punct, cc_other);
  constant origin : point_t := (0, 0);
  constant empty_ints : int_vec(1 to 0) := (others => 0);
  constant magic_number : integer;
  constant version_text : string;
  type sev_label_t is array (severity_level) of string(1 to 7);
  constant sev_labels : sev_label_t :=
    ("note   ", "warning", "error  ", "failure");
end package bl_types;

package body bl_types is
  constant magic_number : integer := 16#BEEF#;
  constant version_text : string := "biglib-1.4";
end package body bl_types;

-- Character classification and conversion.
library ieee;
use ieee.std_logic_1164.all;
use work.bl_types.all;

package bl_chars is
  function is_digit (c : character) return boolean;
  function is_upper (c : character) return boolean;
  function is_lower (c : character) return boolean;
  function is_alpha (c : character) return boolean;
  function is_alnum (c : character) return boolean;
  function is_space (c : character) return boolean;
  function is_hex (c : character) return boolean;
  function is_print (c : character) return boolean;
  function to_upper (c : character) return character;
  function to_lower (c : character) return character;
  function digit_value (c : character) return integer;
  function hex_value (c : character) return integer;
  function hex_char (n : natural range 0 to 15) return character;
  function hex_char (n : natural range 0 to 15; upper : boolean) return character;
  function classify (c : character) return char_class_t;
  function to_sl (c : character) return std_ulogic;
  function to_char (s : std_ulogic) return character;
  function is_vowel (c : character) return boolean;
  function rot (c : character; n : integer) return character;
  function next_char (c : character) return character;
  function char_code (c : character) return natural;
  function code_char (n : natural) return character;
  alias is_whitespace is is_space [character return boolean];
end package bl_chars;

package body bl_chars is
  function is_digit (c : character) return boolean is
  begin
    return c >= '0' and c <= '9';
  end function;

  function is_upper (c : character) return boolean is
  begin
    return c >= 'A' and c <= 'Z';
  end function;

  function is_lower (c : character) return boolean is
  begin
    return c >= 'a' and c <= 'z';
  end function;

  function is_alpha (c : character) return boolean is
  begin
    return is_upper(c) or is_lower(c);
  end function;

  function is_alnum (c : character) return boolean is
  begin
    return is_alpha(c) or is_digit(c);
  end function;

  function is_space (c : character) return boolean is
  begin
    return c = ' ' or c = HT or c = LF or c = CR;
  end function;

  function is_hex (c : character) return boolean is
  begin
    return is_digit(c) or (c >= 'a' and c <= 'f') or (c >= 'A' and c <= 'F');
  end function;

  function is_print (c : character) return boolean is
  begin
    return character'pos(c) >= 32 and character'pos(c) < 127;
  end function;

  function to_upper (c : character) return character is
  begin
    if is_lower(c) then
      return character'val(character'pos(c) - 32);
    end if;
    return c;
  end function;

  function to_lower (c : character) return character is
  begin
    if is_upper(c) then
      return character'val(character'pos(c) + 32);
    end if;
    return c;
  end function;

  function digit_value (c : character) return integer is
  begin
    if is_digit(c) then
      return character'pos(c) - character'pos('0');
    end if;
    return -1;
  end function;

  function hex_value (c : character) return integer is
  begin
    if is_digit(c) then
      return digit_value(c);
    elsif c >= 'a' and c <= 'f' then
      return character'pos(c) - character'pos('a') + 10;
    elsif c >= 'A' and c <= 'F' then
      return character'pos(c) - character'pos('A') + 10;
    end if;
    return -1;
  end function;

  function hex_char (n : natural range 0 to 15) return character is
    constant tab : string(1 to 16) := "0123456789abcdef";
  begin
    return tab(n + 1);
  end function;

  function hex_char (n : natural range 0 to 15; upper : boolean) return character is
  begin
    if upper then
      return to_upper(hex_char(n));
    end if;
    return hex_char(n);
  end function;

  function classify (c : character) return char_class_t is
  begin
    if character'pos(c) < 32 and c /= HT then
      return cc_control;
    elsif is_space(c) then
      return cc_space;
    elsif is_digit(c) then
      return cc_digit;
    elsif is_upper(c) then
      return cc_upper;
    elsif is_lower(c) then
      return cc_lower;
    elsif is_print(c) then
      return cc_punct;
    end if;
    return cc_other;
  end function;

  function to_sl (c : character) return std_ulogic is
  begin
    case c is
      when '0' => return '0';
      when '1' => return '1';
      when 'Z' | 'z' => return 'Z';
      when 'X' | 'x' => return 'X';
      when 'U' | 'u' => return 'U';
      when 'W' | 'w' => return 'W';
      when 'L' | 'l' => return 'L';
      when 'H' | 'h' => return 'H';
      when '-' => return '-';
      when others => return 'X';
    end case;
  end function;

  function to_char (s : std_ulogic) return character is
  begin
    case s is
      when '0' => return '0';
      when '1' => return '1';
      when 'Z' => return 'Z';
      when 'U' => return 'U';
      when 'W' => return 'W';
      when 'L' => return 'L';
      when 'H' => return 'H';
      when '-' => return '-';
      when others => return 'X';
    end case;
  end function;

  function is_vowel (c : character) return boolean is
  begin
    case to_lower(c) is
      when 'a' | 'e' | 'i' | 'o' | 'u' => return true;
      when others => return false;
    end case;
  end function;

  function rot (c : character; n : integer) return character is
    variable base : integer;
  begin
    if is_upper(c) then
      base := character'pos('A');
    elsif is_lower(c) then
      base := character'pos('a');
    else
      return c;
    end if;
    return character'val(base + ((character'pos(c) - base + n) mod 26));
  end function;

  function next_char (c : character) return character is
  begin
    if c = character'high then
      return character'low;
    end if;
    return character'succ(c);
  end function;

  function char_code (c : character) return natural is
  begin
    return character'pos(c);
  end function;

  function code_char (n : natural) return character is
  begin
    return character'val(n mod 256);
  end function;
end package body bl_chars;

-- String utilities with several overloads.
library ieee;
use ieee.std_logic_1164.all;
use work.bl_types.all;
use work.bl_chars.all;

package bl_strings is
  function upper (s : string) return string;
  function lower (s : string) return string;
  function trim (s : string) return string;
  function trim_left (s : string) return string;
  function trim_right (s : string) return string;
  function trim (s : string; c : character) return string;
  function starts_with (s, prefix : string) return boolean;
  function ends_with (s, suffix : string) return boolean;
  function contains (s : string; c : character) return boolean;
  function contains (s, sub : string) return boolean;
  function index_of (s : string; c : character; from : positive := 1) return natural;
  function index_of (s, sub : string) return natural;
  function count (s : string; c : character) return natural;
  function count (s, sub : string) return natural;
  function replace (s : string; a, b : character) return string;
  function replace (s, a, b : string) return string;
  function reverse (s : string) return string;
  function repeat (s : string; n : natural) return string;
  function repeat (c : character; n : natural) return string;
  function pad_left (s : string; w : natural; c : character := ' ') return string;
  function pad_right (s : string; w : natural; c : character := ' ') return string;
  function center (s : string; w : natural) return string;
  function substr (s : string; first : positive; len : natural) return string;
  function head (s : string; n : natural) return string;
  function tail (s : string; n : natural) return string;
  function field_count (s : string; sep : character) return natural;
  function field (s : string; sep : character; n : positive) return string;
  function join (v : string_vec; sep : string) return string;
  function join (v : string_vec; sep : character) return string;
  function is_number (s : string) return boolean;
  function is_blank (s : string) return boolean;
  function equal_nocase (a, b : string) return boolean;
  function compare (a, b : string) return integer;
  function capitalize (s : string) return string;
  function squeeze (s : string) return string;
  function rot13 (s : string) return string;
  function hash (s : string) return natural;
  function vowels (s : string) return natural;
  function longest (v : string_vec) return natural;
  function to_slv_str (s : string) return std_logic_vector;
  function from_slv_str (v : std_logic_vector) return string;
  function words (s : string) return natural;
  function abbreviate (s : string; maxlen : positive) return string;
  function quote (s : string) return string;
  function unquote (s : string) return string;
  function plural (n : integer; one : string; many : string) return string;
end package bl_strings;

package body bl_strings is
  function upper (s : string) return string is
    variable r : string(s'range);
  begin
    for i in s'range loop
      r(i) := to_upper(s(i));
    end loop;
    return r;
  end function;

  function lower (s : string) return string is
    variable r : string(s'range);
  begin
    for i in s'range loop
      r(i) := to_lower(s(i));
    end loop;
    return r;
  end function;

  function trim_left (s : string) return string is
  begin
    for i in s'range loop
      if not is_space(s(i)) then
        return s(i to s'right);
      end if;
    end loop;
    return "";
  end function;

  function trim_right (s : string) return string is
  begin
    for i in s'reverse_range loop
      if not is_space(s(i)) then
        return s(s'left to i);
      end if;
    end loop;
    return "";
  end function;

  function trim (s : string) return string is
  begin
    return trim_left(trim_right(s));
  end function;

  function trim (s : string; c : character) return string is
    variable lo : integer := s'left;
    variable hi : integer := s'right;
  begin
    while lo <= hi and s(lo) = c loop
      lo := lo + 1;
    end loop;
    while hi >= lo and s(hi) = c loop
      hi := hi - 1;
    end loop;
    return s(lo to hi);
  end function;

  function starts_with (s, prefix : string) return boolean is
  begin
    if prefix'length > s'length then
      return false;
    end if;
    return s(s'left to s'left + prefix'length - 1) = prefix;
  end function;

  function ends_with (s, suffix : string) return boolean is
  begin
    if suffix'length > s'length then
      return false;
    end if;
    return s(s'right - suffix'length + 1 to s'right) = suffix;
  end function;

  function index_of (s : string; c : character; from : positive := 1) return natural is
  begin
    for i in from to s'length loop
      if s(s'left + i - 1) = c then
        return i;
      end if;
    end loop;
    return 0;
  end function;

  function index_of (s, sub : string) return natural is
  begin
    if sub'length = 0 or sub'length > s'length then
      return 0;
    end if;
    for i in 1 to s'length - sub'length + 1 loop
      if s(s'left + i - 1 to s'left + i + sub'length - 2) = sub then
        return i;
      end if;
    end loop;
    return 0;
  end function;

  function contains (s : string; c : character) return boolean is
  begin
    return index_of(s, c) /= 0;
  end function;

  function contains (s, sub : string) return boolean is
  begin
    return index_of(s, sub) /= 0;
  end function;

  function count (s : string; c : character) return natural is
    variable n : natural := 0;
  begin
    for i in s'range loop
      if s(i) = c then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function count (s, sub : string) return natural is
    variable n : natural := 0;
    variable pos : natural := 1;
    variable k : natural;
  begin
    if sub'length = 0 then
      return 0;
    end if;
    while pos <= s'length loop
      k := index_of(s(s'left + pos - 1 to s'right), sub);
      exit when k = 0;
      n := n + 1;
      pos := pos + k + sub'length - 1;
    end loop;
    return n;
  end function;

  function replace (s : string; a, b : character) return string is
    variable r : string(s'range) := s;
  begin
    for i in r'range loop
      if r(i) = a then
        r(i) := b;
      end if;
    end loop;
    return r;
  end function;

  function replace (s, a, b : string) return string is
    variable k : natural;
  begin
    if a'length = 0 then
      return s;
    end if;
    k := index_of(s, a);
    if k = 0 then
      return s;
    end if;
    return s(s'left to s'left + k - 2) & b & replace(s(s'left + k - 1 + a'length to s'right), a, b);
  end function;

  function reverse (s : string) return string is
    variable r : string(1 to s'length);
    variable j : natural := s'length;
  begin
    for i in s'range loop
      r(j) := s(i);
      j := j - 1;
    end loop;
    return r;
  end function;

  function repeat (s : string; n : natural) return string is
  begin
    if n = 0 then
      return "";
    end if;
    return s & repeat(s, n - 1);
  end function;

  function repeat (c : character; n : natural) return string is
    variable r : string(1 to n) := (others => c);
  begin
    return r;
  end function;

  function pad_left (s : string; w : natural; c : character := ' ') return string is
  begin
    if s'length >= w then
      return s;
    end if;
    return repeat(c, w - s'length) & s;
  end function;

  function pad_right (s : string; w : natural; c : character := ' ') return string is
  begin
    if s'length >= w then
      return s;
    end if;
    return s & repeat(c, w - s'length);
  end function;

  function center (s : string; w : natural) return string is
    variable left : natural;
  begin
    if s'length >= w then
      return s;
    end if;
    left := (w - s'length) / 2;
    return repeat(' ', left) & s & repeat(' ', w - s'length - left);
  end function;

  function substr (s : string; first : positive; len : natural) return string is
    variable lo : integer := s'left + first - 1;
    variable hi : integer := lo + len - 1;
  begin
    if hi > s'right then
      hi := s'right;
    end if;
    return s(lo to hi);
  end function;

  function head (s : string; n : natural) return string is
  begin
    if n >= s'length then
      return s;
    end if;
    return s(s'left to s'left + n - 1);
  end function;

  function tail (s : string; n : natural) return string is
  begin
    if n >= s'length then
      return s;
    end if;
    return s(s'right - n + 1 to s'right);
  end function;

  function field_count (s : string; sep : character) return natural is
  begin
    return count(s, sep) + 1;
  end function;

  function field (s : string; sep : character; n : positive) return string is
    variable start : integer := s'left;
    variable k : natural := 1;
  begin
    for i in s'range loop
      if s(i) = sep then
        if k = n then
          return s(start to i - 1);
        end if;
        k := k + 1;
        start := i + 1;
      end if;
    end loop;
    if k = n then
      return s(start to s'right);
    end if;
    return "";
  end function;

  function join (v : string_vec; sep : string) return string is
  begin
    if v'length = 0 then
      return "";
    elsif v'length = 1 then
      return v(v'left);
    end if;
    return v(v'left) & sep & join(v(v'left + 1 to v'right), sep);
  end function;

  function join (v : string_vec; sep : character) return string is
  begin
    return join(v, string'(1 => sep));
  end function;

  function is_number (s : string) return boolean is
    variable first : integer := s'left;
  begin
    if s'length = 0 then
      return false;
    end if;
    if s(first) = '-' or s(first) = '+' then
      first := first + 1;
      if first > s'right then
        return false;
      end if;
    end if;
    for i in first to s'right loop
      if not is_digit(s(i)) and s(i) /= '_' then
        return false;
      end if;
    end loop;
    return true;
  end function;

  function is_blank (s : string) return boolean is
  begin
    for i in s'range loop
      if not is_space(s(i)) then
        return false;
      end if;
    end loop;
    return true;
  end function;

  function equal_nocase (a, b : string) return boolean is
  begin
    return upper(a) = upper(b);
  end function;

  function compare (a, b : string) return integer is
    constant n : natural := minimum(a'length, b'length);
  begin
    for i in 0 to n - 1 loop
      if a(a'left + i) < b(b'left + i) then
        return -1;
      elsif a(a'left + i) > b(b'left + i) then
        return 1;
      end if;
    end loop;
    if a'length < b'length then
      return -1;
    elsif a'length > b'length then
      return 1;
    end if;
    return 0;
  end function;

  function capitalize (s : string) return string is
    variable r : string(s'range) := lower(s);
    variable at_start : boolean := true;
  begin
    for i in r'range loop
      if at_start and is_alpha(r(i)) then
        r(i) := to_upper(r(i));
      end if;
      at_start := is_space(r(i));
    end loop;
    return r;
  end function;

  function squeeze (s : string) return string is
    variable r : string(1 to s'length);
    variable n : natural := 0;
    variable prev_space : boolean := false;
  begin
    for i in s'range loop
      if is_space(s(i)) then
        if not prev_space then
          n := n + 1;
          r(n) := ' ';
        end if;
        prev_space := true;
      else
        n := n + 1;
        r(n) := s(i);
        prev_space := false;
      end if;
    end loop;
    return r(1 to n);
  end function;

  function rot13 (s : string) return string is
    variable r : string(s'range);
  begin
    for i in s'range loop
      r(i) := rot(s(i), 13);
    end loop;
    return r;
  end function;

  function hash (s : string) return natural is
    variable h : natural := 5381;
  begin
    for i in s'range loop
      h := (h * 33 + character'pos(s(i))) mod 1000003;
    end loop;
    return h;
  end function;

  function vowels (s : string) return natural is
    variable n : natural := 0;
  begin
    for i in s'range loop
      if is_vowel(s(i)) then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function longest (v : string_vec) return natural is
    variable m : natural := 0;
  begin
    for i in v'range loop
      m := maximum(m, v(i)'length);
    end loop;
    return m;
  end function;

  function to_slv_str (s : string) return std_logic_vector is
    variable r : std_logic_vector(1 to s'length);
    variable j : natural := 1;
  begin
    for i in s'range loop
      r(j) := to_sl(s(i));
      j := j + 1;
    end loop;
    return r;
  end function;

  function from_slv_str (v : std_logic_vector) return string is
    variable r : string(1 to v'length);
    variable j : natural := 1;
  begin
    for i in v'range loop
      r(j) := to_char(v(i));
      j := j + 1;
    end loop;
    return r;
  end function;

  function words (s : string) return natural is
    variable n : natural := 0;
    variable inword : boolean := false;
  begin
    for i in s'range loop
      if is_space(s(i)) then
        inword := false;
      elsif not inword then
        inword := true;
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function abbreviate (s : string; maxlen : positive) return string is
  begin
    if s'length <= maxlen then
      return s;
    elsif maxlen <= 3 then
      return head(s, maxlen);
    end if;
    return head(s, maxlen - 3) & "...";
  end function;

  function quote (s : string) return string is
  begin
    return '"' & s & '"';
  end function;

  function unquote (s : string) return string is
  begin
    if s'length >= 2 and s(s'left) = '"' and s(s'right) = '"' then
      return s(s'left + 1 to s'right - 1);
    end if;
    return s;
  end function;

  function plural (n : integer; one : string; many : string) return string is
  begin
    if n = 1 or n = -1 then
      return integer'image(n) & " " & one;
    end if;
    return integer'image(n) & " " & many;
  end function;
end package body bl_strings;

-- Conversions between the standard numeric, logic, time and boolean types.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
use work.bl_types.all;
use work.bl_chars.all;

package bl_conv is
  function to_str (v : integer) return string;
  function to_str (v : boolean) return string;
  function to_str (v : real; digits : natural := 3) return string;
  function to_str (v : time) return string;
  function to_str (v : std_ulogic) return string;
  function to_str (v : std_logic_vector) return string;
  function to_str (v : unsigned) return string;
  function to_str (v : signed) return string;
  function to_str (v : bit_vector) return string;
  function to_str (v : character) return string;
  function to_str (v : int_vec) return string;
  function to_str (v : bool_vec) return string;
  function to_str (v : real_vec; digits : natural := 2) return string;
  function to_hex (v : std_logic_vector) return string;
  function to_hex (v : unsigned) return string;
  function to_hex (v : natural; digits : positive) return string;
  function to_bin (v : natural; width : positive) return string;
  function to_int (s : string) return integer;
  function to_int (b : boolean) return integer;
  function to_int (b : bit) return integer;
  function to_int (v : std_ulogic) return integer;
  function to_int (v : std_logic_vector) return integer;
  function to_int (v : bit_vector) return natural;
  function to_bool (s : string) return boolean;
  function to_bool (i : integer) return boolean;
  function to_bool (v : std_ulogic) return boolean;
  function to_sl (b : boolean) return std_ulogic;
  function to_sl (i : integer) return std_ulogic;
  function to_slv (v : integer; width : positive) return std_logic_vector;
  function to_slv (v : unsigned) return std_logic_vector;
  function to_slv (v : signed) return std_logic_vector;
  function to_slv (s : string) return std_logic_vector;
  function to_slv (v : bool_vec) return std_logic_vector;
  function to_uns (v : integer; width : positive) return unsigned;
  function to_uns (v : std_logic_vector) return unsigned;
  function to_uns (v : signed) return unsigned;
  function to_sgn (v : integer; width : positive) return signed;
  function to_sgn (v : std_logic_vector) return signed;
  function to_sgn (v : unsigned) return signed;
  function to_bitvec (v : std_logic_vector) return bit_vector;
  function to_bitvec (v : natural; width : positive) return bit_vector;
  function to_real (v : integer) return real;
  function to_real (v : time) return real;
  function to_real (s : string) return real;
  function to_time_ns (v : real) return time;
  function to_time_ns (v : integer) return time;
  function to_bool_vec (v : std_logic_vector) return bool_vec;
  function to_ints (v : std_logic_vector) return int_vec;
  function to_reals (v : int_vec) return real_vec;
  function round_to_int (v : real) return integer;
  function parse_hex (s : string) return std_logic_vector;
  function parse_bin (s : string) return std_logic_vector;
  function sat_resize (v : signed; width : positive) return signed;
  function bytes_of (v : std_logic_vector) return byte_array;
  function word_of (b : byte_array) return std_logic_vector;
  function swap_bytes (v : std_logic_vector) return std_logic_vector;
end package bl_conv;

package body bl_conv is
  function to_str (v : integer) return string is
  begin
    return integer'image(v);
  end function;

  function to_str (v : boolean) return string is
  begin
    if v then
      return "true";
    end if;
    return "false";
  end function;

  function to_str (v : real; digits : natural := 3) return string is
    variable a : real := abs v;
    variable scale : real := 10.0 ** digits;
    variable total : integer := integer(round(a * scale));
    variable ip : integer := total / integer(scale);
    variable fp : integer := total mod integer(scale);
    variable ftxt : string(1 to digits) := (others => '0');
    variable sign : string(1 to 1) := "-";
  begin
    for i in digits downto 1 loop
      ftxt(i) := character'val(character'pos('0') + fp mod 10);
      fp := fp / 10;
    end loop;
    if v >= 0.0 or total = 0 then
      sign := "+";
    end if;
    if digits = 0 then
      return sign & integer'image(ip);
    end if;
    return sign & integer'image(ip) & "." & ftxt;
  end function;

  function to_str (v : time) return string is
  begin
    return time'image(v);
  end function;

  function to_str (v : std_ulogic) return string is
  begin
    return (1 => to_char(v));
  end function;

  function to_str (v : std_logic_vector) return string is
    variable r : string(1 to v'length);
    variable j : natural := 1;
  begin
    for i in v'range loop
      r(j) := to_char(v(i));
      j := j + 1;
    end loop;
    return r;
  end function;

  function to_str (v : unsigned) return string is
  begin
    return to_str(std_logic_vector(v));
  end function;

  function to_str (v : signed) return string is
  begin
    return to_str(std_logic_vector(v));
  end function;

  function to_str (v : bit_vector) return string is
    variable r : string(1 to v'length);
    variable j : natural := 1;
  begin
    for i in v'range loop
      if v(i) = '1' then
        r(j) := '1';
      else
        r(j) := '0';
      end if;
      j := j + 1;
    end loop;
    return r;
  end function;

  function to_str (v : character) return string is
  begin
    return (1 => v);
  end function;

  function to_str (v : int_vec) return string is
    variable acc : string(1 to 400);
    variable n : natural := 1;
  begin
    acc(1) := '[';
    for i in v'range loop
      if i /= v'left then
        n := n + 1;
        acc(n) := ',';
      end if;
      acc(n + 1 to n + integer'image(v(i))'length) := integer'image(v(i));
      n := n + integer'image(v(i))'length;
    end loop;
    acc(n + 1) := ']';
    return acc(1 to n + 1);
  end function;

  function to_str (v : bool_vec) return string is
    variable r : string(1 to v'length + 2);
    variable j : natural := 2;
  begin
    r(1) := '<';
    for i in v'range loop
      if v(i) then
        r(j) := 'T';
      else
        r(j) := 'F';
      end if;
      j := j + 1;
    end loop;
    r(j) := '>';
    return r;
  end function;

  function to_str (v : real_vec; digits : natural := 2) return string is
  begin
    if v'length = 0 then
      return "{}";
    elsif v'length = 1 then
      return "{" & to_str(v(v'left), digits) & "}";
    end if;
    return "{" & to_str(v(v'left), digits) & "," & to_str(v(v'left + 1), digits) & "...}";
  end function;

  function to_hex (v : std_logic_vector) return string is
    constant padded : std_logic_vector(1 to ((v'length + 3) / 4) * 4) := (1 to ((v'length + 3) / 4) * 4 - v'length => '0') & v;
    variable r : string(1 to padded'length / 4);
    variable n : natural;
  begin
    for i in r'range loop
      if is_x(padded(4 * i - 3 to 4 * i)) then
        r(i) := 'X';
      else
        n := to_integer(unsigned(padded(4 * i - 3 to 4 * i)));
        r(i) := hex_char(n, true);
      end if;
    end loop;
    return r;
  end function;

  function to_hex (v : unsigned) return string is
  begin
    return to_hex(std_logic_vector(v));
  end function;

  function to_hex (v : natural; digits : positive) return string is
  begin
    return to_hex(std_logic_vector(to_unsigned(v, digits * 4)));
  end function;

  function to_bin (v : natural; width : positive) return string is
  begin
    return to_str(to_unsigned(v, width));
  end function;

  function to_int (s : string) return integer is
    variable r : integer := 0;
    variable neg : boolean := false;
  begin
    for i in s'range loop
      if s(i) = '-' and i = s'left then
        neg := true;
      elsif is_digit(s(i)) then
        r := r * 10 + digit_value(s(i));
      end if;
    end loop;
    if neg then
      return -r;
    end if;
    return r;
  end function;

  function to_int (b : boolean) return integer is
  begin
    if b then
      return 1;
    end if;
    return 0;
  end function;

  function to_int (b : bit) return integer is
  begin
    if b = '1' then
      return 1;
    end if;
    return 0;
  end function;

  function to_int (v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' then
      return 1;
    end if;
    return 0;
  end function;

  function to_int (v : std_logic_vector) return integer is
  begin
    return to_integer(unsigned(v));
  end function;

  function to_int (v : bit_vector) return natural is
    variable r : natural := 0;
  begin
    for i in v'range loop
      r := r * 2 + to_int(v(i));
    end loop;
    return r;
  end function;

  function to_bool (s : string) return boolean is
  begin
    return s = "true" or s = "TRUE" or s = "1" or s = "yes";
  end function;

  function to_bool (i : integer) return boolean is
  begin
    return i /= 0;
  end function;

  function to_bool (v : std_ulogic) return boolean is
  begin
    return v = '1' or v = 'H';
  end function;

  function to_sl (b : boolean) return std_ulogic is
  begin
    if b then
      return '1';
    end if;
    return '0';
  end function;

  function to_sl (i : integer) return std_ulogic is
  begin
    if i /= 0 then
      return '1';
    end if;
    return '0';
  end function;

  function to_slv (v : integer; width : positive) return std_logic_vector is
  begin
    if v < 0 then
      return std_logic_vector(to_signed(v, width));
    end if;
    return std_logic_vector(to_unsigned(v, width));
  end function;

  function to_slv (v : unsigned) return std_logic_vector is
  begin
    return std_logic_vector(v);
  end function;

  function to_slv (v : signed) return std_logic_vector is
  begin
    return std_logic_vector(v);
  end function;

  function to_slv (s : string) return std_logic_vector is
    variable r : std_logic_vector(1 to s'length);
    variable j : natural := 1;
  begin
    for i in s'range loop
      r(j) := to_sl(s(i));
      j := j + 1;
    end loop;
    return r;
  end function;

  function to_slv (v : bool_vec) return std_logic_vector is
    variable r : std_logic_vector(v'range);
  begin
    for i in v'range loop
      r(i) := to_sl(v(i));
    end loop;
    return r;
  end function;

  function to_uns (v : integer; width : positive) return unsigned is
  begin
    return to_unsigned(v, width);
  end function;

  function to_uns (v : std_logic_vector) return unsigned is
  begin
    return unsigned(v);
  end function;

  function to_uns (v : signed) return unsigned is
  begin
    return unsigned(v);
  end function;

  function to_sgn (v : integer; width : positive) return signed is
  begin
    return to_signed(v, width);
  end function;

  function to_sgn (v : std_logic_vector) return signed is
  begin
    return signed(v);
  end function;

  function to_sgn (v : unsigned) return signed is
  begin
    return signed(v);
  end function;

  function to_bitvec (v : std_logic_vector) return bit_vector is
    variable r : bit_vector(v'range);
  begin
    for i in v'range loop
      if v(i) = '1' or v(i) = 'H' then
        r(i) := '1';
      else
        r(i) := '0';
      end if;
    end loop;
    return r;
  end function;

  function to_bitvec (v : natural; width : positive) return bit_vector is
  begin
    return to_bitvec(std_logic_vector(to_unsigned(v, width)));
  end function;

  function to_real (v : integer) return real is
  begin
    return real(v);
  end function;

  function to_real (v : time) return real is
  begin
    return real(v / 1 ps) / 1000.0;
  end function;

  function to_real (s : string) return real is
    variable ip : integer := 0;
    variable fp : real := 0.0;
    variable scale : real := 0.1;
    variable seen_dot : boolean := false;
    variable neg : boolean := false;
  begin
    for i in s'range loop
      if s(i) = '-' then
        neg := true;
      elsif s(i) = '.' then
        seen_dot := true;
      elsif is_digit(s(i)) then
        if seen_dot then
          fp := fp + scale * real(digit_value(s(i)));
          scale := scale / 10.0;
        else
          ip := ip * 10 + digit_value(s(i));
        end if;
      end if;
    end loop;
    if neg then
      return -(real(ip) + fp);
    end if;
    return real(ip) + fp;
  end function;

  function to_time_ns (v : real) return time is
  begin
    return integer(round(v * 1000.0)) * 1 ps;
  end function;

  function to_time_ns (v : integer) return time is
  begin
    return v * 1 ns;
  end function;

  function to_bool_vec (v : std_logic_vector) return bool_vec is
    variable r : bool_vec(v'range);
  begin
    for i in v'range loop
      r(i) := to_bool(v(i));
    end loop;
    return r;
  end function;

  function to_ints (v : std_logic_vector) return int_vec is
    variable r : int_vec(v'range);
  begin
    for i in v'range loop
      r(i) := to_int(v(i));
    end loop;
    return r;
  end function;

  function to_reals (v : int_vec) return real_vec is
    variable r : real_vec(v'range);
  begin
    for i in v'range loop
      r(i) := real(v(i));
    end loop;
    return r;
  end function;

  function round_to_int (v : real) return integer is
  begin
    return integer(round(v));
  end function;

  function parse_hex (s : string) return std_logic_vector is
    variable r : std_logic_vector(1 to 4 * s'length);
    variable n : integer;
    variable j : natural := 0;
  begin
    for i in s'range loop
      n := hex_value(s(i));
      if n < 0 then
        r(4 * j + 1 to 4 * j + 4) := "XXXX";
      else
        r(4 * j + 1 to 4 * j + 4) := std_logic_vector(to_unsigned(n, 4));
      end if;
      j := j + 1;
    end loop;
    return r;
  end function;

  function parse_bin (s : string) return std_logic_vector is
  begin
    return to_slv(s);
  end function;

  function sat_resize (v : signed; width : positive) return signed is
    constant hi : signed(width - 1 downto 0) := '0' & (width - 2 downto 0 => '1');
    constant lo : signed(width - 1 downto 0) := '1' & (width - 2 downto 0 => '0');
  begin
    if v'length <= width then
      return resize(v, width);
    elsif v > resize(hi, v'length) then
      return hi;
    elsif v < resize(lo, v'length) then
      return lo;
    end if;
    return resize(v, width);
  end function;

  function bytes_of (v : std_logic_vector) return byte_array is
    constant n : natural := v'length / 8;
    variable r : byte_array(0 to n - 1);
    variable norm : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    for i in 0 to n - 1 loop
      r(i) := norm(8 * i + 7 downto 8 * i);
    end loop;
    return r;
  end function;

  function word_of (b : byte_array) return std_logic_vector is
    variable r : std_logic_vector(8 * b'length - 1 downto 0);
    variable j : natural := 0;
  begin
    for i in b'range loop
      r(8 * j + 7 downto 8 * j) := b(i);
      j := j + 1;
    end loop;
    return r;
  end function;

  function swap_bytes (v : std_logic_vector) return std_logic_vector is
    constant b : byte_array := bytes_of(v);
    variable r : byte_array(b'range);
  begin
    for i in b'range loop
      r(b'high - i) := b(i);
    end loop;
    return word_of(r);
  end function;
end package body bl_conv;

-- Operations on integer, real and boolean vectors; functions returning unconstrained arrays.
library ieee;
use ieee.math_real.all;
use work.bl_types.all;

package bl_vec is
  function sum (v : int_vec) return integer;
  function sum (v : real_vec) return real;
  function product (v : int_vec) return integer;
  function max_of (v : int_vec) return integer;
  function min_of (v : int_vec) return integer;
  function max_of (v : real_vec) return real;
  function min_of (v : real_vec) return real;
  function mean (v : real_vec) return real;
  function mean (v : int_vec) return real;
  function variance (v : real_vec) return real;
  function index_of (v : int_vec; x : integer) return integer;
  function contains (v : int_vec; x : integer) return boolean;
  function count_true (v : bool_vec) return natural;
  function any_true (v : bool_vec) return boolean;
  function all_true (v : bool_vec) return boolean;
  function reverse (v : int_vec) return int_vec;
  function sorted (v : int_vec) return int_vec;
  function is_sorted (v : int_vec) return boolean;
  function iota (n : natural; first : integer := 0) return int_vec;
  function span (lo, hi : integer; step : positive := 1) return int_vec;
  function cumsum (v : int_vec) return int_vec;
  function diffs (v : int_vec) return int_vec;
  function positives (v : int_vec) return int_vec;
  function unique (v : int_vec) return int_vec;
  function scale (v : int_vec; k : integer) return int_vec;
  function dot (a, b : int_vec) return integer;
  function median (v : int_vec) return integer;
  function "+" (a, b : int_vec) return int_vec;
  function "-" (a, b : int_vec) return int_vec;
  function "*" (a : int_vec; k : integer) return int_vec;
  function "+" (a : int_vec; k : integer) return int_vec;
  function "-" (a : int_vec) return int_vec;
  function "<" (a, b : int_vec) return boolean;
  function "<=" (a, b : int_vec) return boolean;
  function ">" (a, b : int_vec) return boolean;
  function "and" (a, b : bool_vec) return bool_vec;
  function "or" (a, b : bool_vec) return bool_vec;
  function "not" (a : bool_vec) return bool_vec;
end package bl_vec;

package body bl_vec is
  function sum (v : int_vec) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;

  function sum (v : real_vec) return real is
    variable s : real := 0.0;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;

  function product (v : int_vec) return integer is
    variable p : integer := 1;
  begin
    for i in v'range loop
      p := p * v(i);
    end loop;
    return p;
  end function;

  function max_of (v : int_vec) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop
      m := maximum(m, v(i));
    end loop;
    return m;
  end function;

  function min_of (v : int_vec) return integer is
    variable m : integer := integer'high;
  begin
    for i in v'range loop
      m := minimum(m, v(i));
    end loop;
    return m;
  end function;

  function max_of (v : real_vec) return real is
    variable m : real := real'low;
  begin
    for i in v'range loop
      m := maximum(m, v(i));
    end loop;
    return m;
  end function;

  function min_of (v : real_vec) return real is
    variable m : real := real'high;
  begin
    for i in v'range loop
      m := minimum(m, v(i));
    end loop;
    return m;
  end function;

  function mean (v : real_vec) return real is
  begin
    if v'length = 0 then
      return 0.0;
    end if;
    return sum(v) / real(v'length);
  end function;

  function mean (v : int_vec) return real is
  begin
    if v'length = 0 then
      return 0.0;
    end if;
    return real(sum(v)) / real(v'length);
  end function;

  function variance (v : real_vec) return real is
    constant m : real := mean(v);
    variable acc : real := 0.0;
  begin
    if v'length = 0 then
      return 0.0;
    end if;
    for i in v'range loop
      acc := acc + (v(i) - m) * (v(i) - m);
    end loop;
    return acc / real(v'length);
  end function;

  function index_of (v : int_vec; x : integer) return integer is
  begin
    for i in v'range loop
      if v(i) = x then
        return i;
      end if;
    end loop;
    return -1;
  end function;

  function contains (v : int_vec; x : integer) return boolean is
  begin
    return index_of(v, x) >= 0;
  end function;

  function count_true (v : bool_vec) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function any_true (v : bool_vec) return boolean is
  begin
    return count_true(v) > 0;
  end function;

  function all_true (v : bool_vec) return boolean is
  begin
    return count_true(v) = v'length;
  end function;

  function reverse (v : int_vec) return int_vec is
    variable r : int_vec(v'range);
  begin
    for i in v'range loop
      r(v'left + v'right - i) := v(i);
    end loop;
    return r;
  end function;

  function sorted (v : int_vec) return int_vec is
    variable r : int_vec(0 to v'length - 1);
    variable t : integer;
    variable j : integer;
  begin
    for i in v'range loop
      r(i - v'left) := v(i);
    end loop;
    for i in 1 to r'high loop
      t := r(i);
      j := i - 1;
      while j >= 0 and r(j) > t loop
        r(j + 1) := r(j);
        j := j - 1;
      end loop;
      r(j + 1) := t;
    end loop;
    return r;
  end function;

  function is_sorted (v : int_vec) return boolean is
  begin
    for i in v'left to v'right - 1 loop
      if v(i) > v(i + 1) then
        return false;
      end if;
    end loop;
    return true;
  end function;

  function iota (n : natural; first : integer := 0) return int_vec is
    variable r : int_vec(0 to n - 1);
  begin
    for i in r'range loop
      r(i) := first + i;
    end loop;
    return r;
  end function;

  function span (lo, hi : integer; step : positive := 1) return int_vec is
    constant n : integer := (hi - lo) / step + 1;
  begin
    if n <= 0 then
      return empty_ints;
    end if;
    return iota(n, lo) * step + (lo - lo * step);
  end function;

  function cumsum (v : int_vec) return int_vec is
    variable r : int_vec(v'range);
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i);
      r(i) := s;
    end loop;
    return r;
  end function;

  function diffs (v : int_vec) return int_vec is
    variable r : int_vec(0 to v'length - 2);
  begin
    for i in r'range loop
      r(i) := v(v'left + i + 1) - v(v'left + i);
    end loop;
    return r;
  end function;

  function positives (v : int_vec) return int_vec is
    variable r : int_vec(0 to v'length - 1);
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) > 0 then
        r(n) := v(i);
        n := n + 1;
      end if;
    end loop;
    return r(0 to n - 1);
  end function;

  function unique (v : int_vec) return int_vec is
    variable r : int_vec(0 to v'length - 1);
    variable n : natural := 0;
  begin
    for i in v'range loop
      if not contains(r(0 to n - 1), v(i)) then
        r(n) := v(i);
        n := n + 1;
      end if;
    end loop;
    return r(0 to n - 1);
  end function;

  function scale (v : int_vec; k : integer) return int_vec is
  begin
    return v * k;
  end function;

  function dot (a, b : int_vec) return integer is
    variable s : integer := 0;
  begin
    assert a'length = b'length report "dot: length mismatch" severity failure;
    for i in 0 to a'length - 1 loop
      s := s + a(a'left + i) * b(b'left + i);
    end loop;
    return s;
  end function;

  function median (v : int_vec) return integer is
    constant s : int_vec := sorted(v);
  begin
    return s(s'length / 2);
  end function;

  function "+" (a, b : int_vec) return int_vec is
    variable r : int_vec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) + b(b'left + i - a'left);
    end loop;
    return r;
  end function;

  function "-" (a, b : int_vec) return int_vec is
    variable r : int_vec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) - b(b'left + i - a'left);
    end loop;
    return r;
  end function;

  function "*" (a : int_vec; k : integer) return int_vec is
    variable r : int_vec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) * k;
    end loop;
    return r;
  end function;

  function "+" (a : int_vec; k : integer) return int_vec is
    variable r : int_vec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) + k;
    end loop;
    return r;
  end function;

  function "-" (a : int_vec) return int_vec is
  begin
    return a * (-1);
  end function;

  function "<" (a, b : int_vec) return boolean is
    constant n : natural := minimum(a'length, b'length);
  begin
    for i in 0 to n - 1 loop
      if a(a'left + i) /= b(b'left + i) then
        return a(a'left + i) < b(b'left + i);
      end if;
    end loop;
    return a'length < b'length;
  end function;

  function "<=" (a, b : int_vec) return boolean is
  begin
    return not (b < a);
  end function;

  function ">" (a, b : int_vec) return boolean is
  begin
    return b < a;
  end function;

  function "and" (a, b : bool_vec) return bool_vec is
    variable r : bool_vec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) and b(b'left + i - a'left);
    end loop;
    return r;
  end function;

  function "or" (a, b : bool_vec) return bool_vec is
    variable r : bool_vec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) or b(b'left + i - a'left);
    end loop;
    return r;
  end function;

  function "not" (a : bool_vec) return bool_vec is
    variable r : bool_vec(a'range);
  begin
    for i in a'range loop
      r(i) := not a(i);
    end loop;
    return r;
  end function;
end package body bl_vec;

-- Records with overloaded operators: points, fractions, complex numbers, intervals.
library ieee;
use ieee.math_real.all;
use work.bl_types.all;

package bl_records is
  type point_vec is array (natural range <>) of point_t;
  type fraction_t is record
    num : integer;
    den : positive;
  end record;
  type complex_t is record
    re : real;
    im : real;
  end record;
  type interval_t is record
    lo : integer;
    hi : integer;
  end record;
  type rgb_t is record
    r, g, b : nat8;
  end record;
  constant unit_x : point_t := (1, 0);
  constant unit_y : point_t := (0, 1);
  constant half : fraction_t := (1, 2);
  constant imag_unit : complex_t := (0.0, 1.0);
  constant black : rgb_t := (0, 0, 0);
  constant white : rgb_t := (255, 255, 255);

  function "+" (a, b : point_t) return point_t;
  function "-" (a, b : point_t) return point_t;
  function "*" (a : point_t; k : integer) return point_t;
  function "-" (a : point_t) return point_t;
  function "&" (a, b : point_t) return point_vec;
  function "<" (a, b : point_t) return boolean;
  function manhattan (a, b : point_t) return natural;
  function to_string (p : point_t) return string;
  function bounding_box (v : point_vec) return interval_t;
  function centroid (v : point_vec) return point_t;

  function "+" (a, b : fraction_t) return fraction_t;
  function "-" (a, b : fraction_t) return fraction_t;
  function "*" (a, b : fraction_t) return fraction_t;
  function "/" (a, b : fraction_t) return fraction_t;
  function "=" (a, b : fraction_t) return boolean;
  function "<" (a, b : fraction_t) return boolean;
  function "<=" (a, b : fraction_t) return boolean;
  function ">" (a, b : fraction_t) return boolean;
  function ">=" (a, b : fraction_t) return boolean;
  function "/=" (a, b : fraction_t) return boolean;
  function "abs" (a : fraction_t) return fraction_t;
  function "-" (a : fraction_t) return fraction_t;
  function reduce (a : fraction_t) return fraction_t;
  function to_fraction (n : integer; d : positive := 1) return fraction_t;
  function to_real (a : fraction_t) return real;
  function to_string (a : fraction_t) return string;

  function "+" (a, b : complex_t) return complex_t;
  function "-" (a, b : complex_t) return complex_t;
  function "*" (a, b : complex_t) return complex_t;
  function "*" (a : complex_t; k : real) return complex_t;
  function "abs" (a : complex_t) return real;
  function conj (a : complex_t) return complex_t;
  function "=" (a, b : complex_t) return boolean;
  function polar (mag, angle : real) return complex_t;
  function arg (a : complex_t) return real;
  function cpow (a : complex_t; n : natural) return complex_t;

  function interval_width (i : interval_t) return integer;
  function contains (i : interval_t; x : integer) return boolean;
  function overlaps (a, b : interval_t) return boolean;
  function "+" (a, b : interval_t) return interval_t;
  function "and" (a, b : interval_t) return interval_t;
  function "or" (a, b : interval_t) return interval_t;
  function "=" (a, b : interval_t) return boolean;
  function is_empty (i : interval_t) return boolean;

  function brightness (c : rgb_t) return natural;
  function mix (a, b : rgb_t) return rgb_t;
  function pack (c : rgb_t) return natural;
  function unpack (n : natural) return rgb_t;
end package bl_records;

package body bl_records is
  function "+" (a, b : point_t) return point_t is
  begin
    return (a.x + b.x, a.y + b.y);
  end function;

  function "-" (a, b : point_t) return point_t is
  begin
    return (a.x - b.x, a.y - b.y);
  end function;

  function "*" (a : point_t; k : integer) return point_t is
  begin
    return (x => a.x * k, y => a.y * k);
  end function;

  function "-" (a : point_t) return point_t is
  begin
    return (-a.x, -a.y);
  end function;

  function "&" (a, b : point_t) return point_vec is
  begin
    return (a, b);
  end function;

  function "<" (a, b : point_t) return boolean is
  begin
    return a.x < b.x or (a.x = b.x and a.y < b.y);
  end function;

  function manhattan (a, b : point_t) return natural is
  begin
    return abs (a.x - b.x) + abs (a.y - b.y);
  end function;

  function to_string (p : point_t) return string is
  begin
    return "(" & integer'image(p.x) & "," & integer'image(p.y) & ")";
  end function;

  function bounding_box (v : point_vec) return interval_t is
    variable lo : integer := integer'high;
    variable hi : integer := integer'low;
  begin
    for i in v'range loop
      lo := minimum(lo, v(i).x);
      hi := maximum(hi, v(i).x);
    end loop;
    return (lo, hi);
  end function;

  function centroid (v : point_vec) return point_t is
    variable sx, sy : integer := 0;
  begin
    for i in v'range loop
      sx := sx + v(i).x;
      sy := sy + v(i).y;
    end loop;
    return (sx / v'length, sy / v'length);
  end function;

  function gcd (a, b : natural) return natural is
  begin
    if b = 0 then
      return a;
    end if;
    return gcd(b, a mod b);
  end function;

  function reduce (a : fraction_t) return fraction_t is
    constant g : natural := gcd(abs a.num, a.den);
  begin
    if g <= 1 then
      return a;
    end if;
    return (a.num / g, a.den / g);
  end function;

  function "+" (a, b : fraction_t) return fraction_t is
  begin
    return reduce((a.num * b.den + b.num * a.den, a.den * b.den));
  end function;

  function "-" (a, b : fraction_t) return fraction_t is
  begin
    return reduce((a.num * b.den - b.num * a.den, a.den * b.den));
  end function;

  function "*" (a, b : fraction_t) return fraction_t is
  begin
    return reduce((a.num * b.num, a.den * b.den));
  end function;

  function "/" (a, b : fraction_t) return fraction_t is
  begin
    if b.num < 0 then
      return reduce((-a.num * b.den, a.den * (-b.num)));
    end if;
    return reduce((a.num * b.den, a.den * b.num));
  end function;

  function "=" (a, b : fraction_t) return boolean is
  begin
    return a.num * b.den = b.num * a.den;
  end function;

  function "/=" (a, b : fraction_t) return boolean is
  begin
    return not (a = b);
  end function;

  function "<" (a, b : fraction_t) return boolean is
  begin
    return a.num * b.den < b.num * a.den;
  end function;

  function "<=" (a, b : fraction_t) return boolean is
  begin
    return a < b or a = b;
  end function;

  function ">" (a, b : fraction_t) return boolean is
  begin
    return b < a;
  end function;

  function ">=" (a, b : fraction_t) return boolean is
  begin
    return b <= a;
  end function;

  function "abs" (a : fraction_t) return fraction_t is
  begin
    return (abs a.num, a.den);
  end function;

  function "-" (a : fraction_t) return fraction_t is
  begin
    return (-a.num, a.den);
  end function;

  function to_fraction (n : integer; d : positive := 1) return fraction_t is
  begin
    return reduce((n, d));
  end function;

  function to_real (a : fraction_t) return real is
  begin
    return real(a.num) / real(a.den);
  end function;

  function to_string (a : fraction_t) return string is
  begin
    return integer'image(a.num) & "/" & integer'image(a.den);
  end function;

  function "+" (a, b : complex_t) return complex_t is
  begin
    return (a.re + b.re, a.im + b.im);
  end function;

  function "-" (a, b : complex_t) return complex_t is
  begin
    return (a.re - b.re, a.im - b.im);
  end function;

  function "*" (a, b : complex_t) return complex_t is
  begin
    return (a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re);
  end function;

  function "*" (a : complex_t; k : real) return complex_t is
  begin
    return (a.re * k, a.im * k);
  end function;

  function "abs" (a : complex_t) return real is
  begin
    return sqrt(a.re * a.re + a.im * a.im);
  end function;

  function conj (a : complex_t) return complex_t is
  begin
    return (a.re, -a.im);
  end function;

  function "=" (a, b : complex_t) return boolean is
  begin
    return abs (a - b) < 1.0e-9;
  end function;

  function polar (mag, angle : real) return complex_t is
  begin
    return (mag * cos(angle), mag * sin(angle));
  end function;

  function arg (a : complex_t) return real is
  begin
    return arctan(a.im, a.re);
  end function;

  function cpow (a : complex_t; n : natural) return complex_t is
    variable r : complex_t := (1.0, 0.0);
  begin
    for i in 1 to n loop
      r := r * a;
    end loop;
    return r;
  end function;

  function interval_width (i : interval_t) return integer is
  begin
    return i.hi - i.lo + 1;
  end function;

  function is_empty (i : interval_t) return boolean is
  begin
    return i.hi < i.lo;
  end function;

  function contains (i : interval_t; x : integer) return boolean is
  begin
    return x >= i.lo and x <= i.hi;
  end function;

  function overlaps (a, b : interval_t) return boolean is
  begin
    return a.lo <= b.hi and b.lo <= a.hi;
  end function;

  function "+" (a, b : interval_t) return interval_t is
  begin
    return (a.lo + b.lo, a.hi + b.hi);
  end function;

  function "and" (a, b : interval_t) return interval_t is
  begin
    return (maximum(a.lo, b.lo), minimum(a.hi, b.hi));
  end function;

  function "or" (a, b : interval_t) return interval_t is
  begin
    return (minimum(a.lo, b.lo), maximum(a.hi, b.hi));
  end function;

  function "=" (a, b : interval_t) return boolean is
  begin
    return (is_empty(a) and is_empty(b)) or (a.lo = b.lo and a.hi = b.hi);
  end function;

  function brightness (c : rgb_t) return natural is
  begin
    return (c.r * 30 + c.g * 59 + c.b * 11) / 100;
  end function;

  function mix (a, b : rgb_t) return rgb_t is
  begin
    return ((a.r + b.r) / 2, (a.g + b.g) / 2, (a.b + b.b) / 2);
  end function;

  function pack (c : rgb_t) return natural is
  begin
    return c.r * 65536 + c.g * 256 + c.b;
  end function;

  function unpack (n : natural) return rgb_t is
  begin
    return (r => (n / 65536) mod 256, g => (n / 256) mod 256, b => n mod 256);
  end function;
end package body bl_records;

-- Functions on the time type.
use work.bl_types.all;

package bl_timeutil is
  function tmin (a, b : time) return time;
  function tmax (a, b : time) return time;
  function limit (t, lo, hi : time) return time;
  function limit_pos (t : time) return time;
  function to_ns (t : time) return integer;
  function to_ps (t : time) return integer;
  function to_us_real (t : time) return real;
  function from_ns (n : integer) return time;
  function from_hz (f : positive) return time;
  function to_hz (period : time) return natural;
  function round_to (t, quantum : time) return time;
  function is_multiple (t, quantum : time) return boolean;
  function half_period (period : time) return time;
  function cycles_in (span, period : time) return natural;
  function scale_time (t : time; k : natural) return time;
  function earliest (v : time_vec) return time;
  function latest (v : time_vec) return time;
  function total (v : time_vec) return time;
  function unit_name (t : time) return string;
end package bl_timeutil;

package body bl_timeutil is
  function tmin (a, b : time) return time is
  begin
    if a < b then
      return a;
    end if;
    return b;
  end function;

  function tmax (a, b : time) return time is
  begin
    if a > b then
      return a;
    end if;
    return b;
  end function;

  function limit (t, lo, hi : time) return time is
  begin
    return tmin(tmax(t, lo), hi);
  end function;

  function limit_pos (t : time) return time is
  begin
    return tmax(t, 0 fs);
  end function;

  function to_ns (t : time) return integer is
  begin
    return t / 1 ns;
  end function;

  function to_ps (t : time) return integer is
  begin
    return t / 1 ps;
  end function;

  function to_us_real (t : time) return real is
  begin
    return real(t / 1 ns) / 1000.0;
  end function;

  function from_ns (n : integer) return time is
  begin
    return n * 1 ns;
  end function;

  function from_hz (f : positive) return time is
  begin
    return 1 sec / f;
  end function;

  function to_hz (period : time) return natural is
  begin
    return 1 sec / period;
  end function;

  function round_to (t, quantum : time) return time is
  begin
    return ((t + quantum / 2) / quantum) * quantum;
  end function;

  function is_multiple (t, quantum : time) return boolean is
  begin
    return t mod quantum = 0 fs;
  end function;

  function half_period (period : time) return time is
  begin
    return period / 2;
  end function;

  function cycles_in (span, period : time) return natural is
  begin
    return span / period;
  end function;

  function scale_time (t : time; k : natural) return time is
  begin
    return t * k;
  end function;

  function earliest (v : time_vec) return time is
    variable m : time := time'high;
  begin
    for i in v'range loop
      m := tmin(m, v(i));
    end loop;
    return m;
  end function;

  function latest (v : time_vec) return time is
    variable m : time := 0 fs;
  begin
    for i in v'range loop
      m := tmax(m, v(i));
    end loop;
    return m;
  end function;

  function total (v : time_vec) return time is
    variable s : time := 0 fs;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;

  function unit_name (t : time) return string is
  begin
    if t >= 1 sec then
      return "s";
    elsif t >= 1 ms then
      return "ms";
    elsif t >= 1 us then
      return "us";
    elsif t >= 1 ns then
      return "ns";
    elsif t >= 1 ps then
      return "ps";
    end if;
    return "fs";
  end function;
end package body bl_timeutil;

-- Enumeration opcode with navigation, conversion and set helpers.
use work.bl_types.all;

package bl_enum_opcode is
  type opcode_t is (
    op_nop, op_load, op_store, op_add,
    op_sub, op_and, op_or, op_xor,
    op_shl, op_shr, op_jmp, op_jz,
    op_jnz, op_call, op_ret, op_halt);
  type opcode_vec is array (natural range <>) of opcode_t;
  type opcode_set is array (opcode_t) of boolean;
  type opcode_table is array (opcode_t) of natural;
  subtype opcode_low_t is opcode_t range op_nop to op_xor;
  constant opcode_count : positive := opcode_t'pos(opcode_t'high) + 1;
  constant opcode_weights : opcode_table := (op_nop => 6, op_load => 3, op_store => 7, op_add => 1, op_sub => 2, op_and => 9, op_or => 2, op_xor => 6, op_shl => 1, op_shr => 9, op_jmp => 4, op_jz => 1, op_jnz => 2, op_call => 7, op_ret => 7, op_halt => 2);
  constant opcode_none : opcode_set := (others => false);

  function next_opcode (x : opcode_t) return opcode_t;
  function prev_opcode (x : opcode_t) return opcode_t;
  function to_string (x : opcode_t) return string;
  function parse_opcode (s : string) return opcode_t;
  function to_int (x : opcode_t) return natural;
  function to_opcode (n : natural) return opcode_t;
  function distance (a, b : opcode_t) return integer;
  function between (x, lo, hi : opcode_t) return boolean;
  function all_opcode_names return string;
  function make_set (v : opcode_vec) return opcode_set;
  function member (x : opcode_t; s : opcode_set) return boolean;
  function members (s : opcode_set) return natural;
  function weight_of (x : opcode_t) return natural;
  function is_first (x : opcode_t) return boolean;
  function is_last (x : opcode_t) return boolean;
  function all_values return opcode_vec;
  function "+" (x : opcode_t; k : integer) return opcode_t;
  function "-" (x : opcode_t; k : integer) return opcode_t;
  function "-" (a, b : opcode_t) return integer;
end package bl_enum_opcode;

package body bl_enum_opcode is
  function next_opcode (x : opcode_t) return opcode_t is
  begin
    if x = opcode_t'high then
      return opcode_t'low;
    end if;
    return opcode_t'succ(x);
  end function;

  function prev_opcode (x : opcode_t) return opcode_t is
  begin
    if x = opcode_t'low then
      return opcode_t'high;
    end if;
    return opcode_t'pred(x);
  end function;

  function to_string (x : opcode_t) return string is
  begin
    return opcode_t'image(x);
  end function;

  function parse_opcode (s : string) return opcode_t is
  begin
    return opcode_t'value(s);
  end function;

  function to_int (x : opcode_t) return natural is
  begin
    return opcode_t'pos(x);
  end function;

  function to_opcode (n : natural) return opcode_t is
  begin
    return opcode_t'val(n mod opcode_count);
  end function;

  function distance (a, b : opcode_t) return integer is
  begin
    return opcode_t'pos(b) - opcode_t'pos(a);
  end function;

  function between (x, lo, hi : opcode_t) return boolean is
  begin
    return x >= lo and x <= hi;
  end function;

  function all_opcode_names return string is
    variable acc : string(1 to 512);
    variable len : natural := 0;
  begin
    for v in opcode_t loop
      if len > 0 then
        len := len + 1;
        acc(len) := ',';
      end if;
      acc(len + 1 to len + opcode_t'image(v)'length) := opcode_t'image(v);
      len := len + opcode_t'image(v)'length;
    end loop;
    return acc(1 to len);
  end function;

  function make_set (v : opcode_vec) return opcode_set is
    variable s : opcode_set := (others => false);
  begin
    for i in v'range loop
      s(v(i)) := true;
    end loop;
    return s;
  end function;

  function member (x : opcode_t; s : opcode_set) return boolean is
  begin
    return s(x);
  end function;

  function members (s : opcode_set) return natural is
    variable c : natural := 0;
  begin
    for v in opcode_t loop
      if s(v) then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  function weight_of (x : opcode_t) return natural is
  begin
    return opcode_weights(x);
  end function;

  function is_first (x : opcode_t) return boolean is
  begin
    return x = opcode_t'left;
  end function;

  function is_last (x : opcode_t) return boolean is
  begin
    return x = opcode_t'right;
  end function;

  function all_values return opcode_vec is
    variable r : opcode_vec(0 to opcode_count - 1);
  begin
    for v in opcode_t loop
      r(opcode_t'pos(v)) := v;
    end loop;
    return r;
  end function;

  function "+" (x : opcode_t; k : integer) return opcode_t is
  begin
    return opcode_t'val(((opcode_t'pos(x) + k) mod opcode_count + opcode_count) mod opcode_count);
  end function;

  function "-" (x : opcode_t; k : integer) return opcode_t is
  begin
    return x + (-k);
  end function;

  function "-" (a, b : opcode_t) return integer is
  begin
    return opcode_t'pos(a) - opcode_t'pos(b);
  end function;

end package body bl_enum_opcode;

-- Enumeration weekday with navigation, conversion and set helpers.
use work.bl_types.all;

package bl_enum_weekday is
  type weekday_t is (
    monday, tuesday, wednesday, thursday,
    friday, saturday, sunday);
  type weekday_vec is array (natural range <>) of weekday_t;
  type weekday_set is array (weekday_t) of boolean;
  type weekday_table is array (weekday_t) of natural;
  subtype weekday_low_t is weekday_t range monday to wednesday;
  constant weekday_count : positive := weekday_t'pos(weekday_t'high) + 1;
  constant weekday_weights : weekday_table := (monday => 4, tuesday => 2, wednesday => 9, thursday => 7, friday => 1, saturday => 2, sunday => 4);
  constant weekday_none : weekday_set := (others => false);

  function next_weekday (x : weekday_t) return weekday_t;
  function prev_weekday (x : weekday_t) return weekday_t;
  function to_string (x : weekday_t) return string;
  function parse_weekday (s : string) return weekday_t;
  function to_int (x : weekday_t) return natural;
  function to_weekday (n : natural) return weekday_t;
  function distance (a, b : weekday_t) return integer;
  function between (x, lo, hi : weekday_t) return boolean;
  function all_weekday_names return string;
  function make_set (v : weekday_vec) return weekday_set;
  function member (x : weekday_t; s : weekday_set) return boolean;
  function members (s : weekday_set) return natural;
  function weight_of (x : weekday_t) return natural;
  function is_first (x : weekday_t) return boolean;
  function is_last (x : weekday_t) return boolean;
  function all_values return weekday_vec;
  function "+" (x : weekday_t; k : integer) return weekday_t;
  function "-" (x : weekday_t; k : integer) return weekday_t;
  function "-" (a, b : weekday_t) return integer;
end package bl_enum_weekday;

package body bl_enum_weekday is
  function next_weekday (x : weekday_t) return weekday_t is
  begin
    if x = weekday_t'high then
      return weekday_t'low;
    end if;
    return weekday_t'succ(x);
  end function;

  function prev_weekday (x : weekday_t) return weekday_t is
  begin
    if x = weekday_t'low then
      return weekday_t'high;
    end if;
    return weekday_t'pred(x);
  end function;

  function to_string (x : weekday_t) return string is
  begin
    return weekday_t'image(x);
  end function;

  function parse_weekday (s : string) return weekday_t is
  begin
    return weekday_t'value(s);
  end function;

  function to_int (x : weekday_t) return natural is
  begin
    return weekday_t'pos(x);
  end function;

  function to_weekday (n : natural) return weekday_t is
  begin
    return weekday_t'val(n mod weekday_count);
  end function;

  function distance (a, b : weekday_t) return integer is
  begin
    return weekday_t'pos(b) - weekday_t'pos(a);
  end function;

  function between (x, lo, hi : weekday_t) return boolean is
  begin
    return x >= lo and x <= hi;
  end function;

  function all_weekday_names return string is
    variable acc : string(1 to 512);
    variable len : natural := 0;
  begin
    for v in weekday_t loop
      if len > 0 then
        len := len + 1;
        acc(len) := ',';
      end if;
      acc(len + 1 to len + weekday_t'image(v)'length) := weekday_t'image(v);
      len := len + weekday_t'image(v)'length;
    end loop;
    return acc(1 to len);
  end function;

  function make_set (v : weekday_vec) return weekday_set is
    variable s : weekday_set := (others => false);
  begin
    for i in v'range loop
      s(v(i)) := true;
    end loop;
    return s;
  end function;

  function member (x : weekday_t; s : weekday_set) return boolean is
  begin
    return s(x);
  end function;

  function members (s : weekday_set) return natural is
    variable c : natural := 0;
  begin
    for v in weekday_t loop
      if s(v) then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  function weight_of (x : weekday_t) return natural is
  begin
    return weekday_weights(x);
  end function;

  function is_first (x : weekday_t) return boolean is
  begin
    return x = weekday_t'left;
  end function;

  function is_last (x : weekday_t) return boolean is
  begin
    return x = weekday_t'right;
  end function;

  function all_values return weekday_vec is
    variable r : weekday_vec(0 to weekday_count - 1);
  begin
    for v in weekday_t loop
      r(weekday_t'pos(v)) := v;
    end loop;
    return r;
  end function;

  function "+" (x : weekday_t; k : integer) return weekday_t is
  begin
    return weekday_t'val(((weekday_t'pos(x) + k) mod weekday_count + weekday_count) mod weekday_count);
  end function;

  function "-" (x : weekday_t; k : integer) return weekday_t is
  begin
    return x + (-k);
  end function;

  function "-" (a, b : weekday_t) return integer is
  begin
    return weekday_t'pos(a) - weekday_t'pos(b);
  end function;

end package body bl_enum_weekday;

-- Enumeration month with navigation, conversion and set helpers.
use work.bl_types.all;

package bl_enum_month is
  type month_t is (
    jan, feb, mar, apr,
    may, jun, jul, aug,
    sep, oct, nov, dec);
  type month_vec is array (natural range <>) of month_t;
  type month_set is array (month_t) of boolean;
  type month_table is array (month_t) of natural;
  subtype month_low_t is month_t range jan to jun;
  constant month_count : positive := month_t'pos(month_t'high) + 1;
  constant month_weights : month_table := (jan => 1, feb => 7, mar => 1, apr => 4, may => 1, jun => 9, jul => 3, aug => 5, sep => 7, oct => 3, nov => 9, dec => 2);
  constant month_none : month_set := (others => false);

  function next_month (x : month_t) return month_t;
  function prev_month (x : month_t) return month_t;
  function to_string (x : month_t) return string;
  function parse_month (s : string) return month_t;
  function to_int (x : month_t) return natural;
  function to_month (n : natural) return month_t;
  function distance (a, b : month_t) return integer;
  function between (x, lo, hi : month_t) return boolean;
  function all_month_names return string;
  function make_set (v : month_vec) return month_set;
  function member (x : month_t; s : month_set) return boolean;
  function members (s : month_set) return natural;
  function weight_of (x : month_t) return natural;
  function is_first (x : month_t) return boolean;
  function is_last (x : month_t) return boolean;
  function all_values return month_vec;
  function "+" (x : month_t; k : integer) return month_t;
  function "-" (x : month_t; k : integer) return month_t;
  function "-" (a, b : month_t) return integer;
end package bl_enum_month;

package body bl_enum_month is
  function next_month (x : month_t) return month_t is
  begin
    if x = month_t'high then
      return month_t'low;
    end if;
    return month_t'succ(x);
  end function;

  function prev_month (x : month_t) return month_t is
  begin
    if x = month_t'low then
      return month_t'high;
    end if;
    return month_t'pred(x);
  end function;

  function to_string (x : month_t) return string is
  begin
    return month_t'image(x);
  end function;

  function parse_month (s : string) return month_t is
  begin
    return month_t'value(s);
  end function;

  function to_int (x : month_t) return natural is
  begin
    return month_t'pos(x);
  end function;

  function to_month (n : natural) return month_t is
  begin
    return month_t'val(n mod month_count);
  end function;

  function distance (a, b : month_t) return integer is
  begin
    return month_t'pos(b) - month_t'pos(a);
  end function;

  function between (x, lo, hi : month_t) return boolean is
  begin
    return x >= lo and x <= hi;
  end function;

  function all_month_names return string is
    variable acc : string(1 to 512);
    variable len : natural := 0;
  begin
    for v in month_t loop
      if len > 0 then
        len := len + 1;
        acc(len) := ',';
      end if;
      acc(len + 1 to len + month_t'image(v)'length) := month_t'image(v);
      len := len + month_t'image(v)'length;
    end loop;
    return acc(1 to len);
  end function;

  function make_set (v : month_vec) return month_set is
    variable s : month_set := (others => false);
  begin
    for i in v'range loop
      s(v(i)) := true;
    end loop;
    return s;
  end function;

  function member (x : month_t; s : month_set) return boolean is
  begin
    return s(x);
  end function;

  function members (s : month_set) return natural is
    variable c : natural := 0;
  begin
    for v in month_t loop
      if s(v) then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  function weight_of (x : month_t) return natural is
  begin
    return month_weights(x);
  end function;

  function is_first (x : month_t) return boolean is
  begin
    return x = month_t'left;
  end function;

  function is_last (x : month_t) return boolean is
  begin
    return x = month_t'right;
  end function;

  function all_values return month_vec is
    variable r : month_vec(0 to month_count - 1);
  begin
    for v in month_t loop
      r(month_t'pos(v)) := v;
    end loop;
    return r;
  end function;

  function "+" (x : month_t; k : integer) return month_t is
  begin
    return month_t'val(((month_t'pos(x) + k) mod month_count + month_count) mod month_count);
  end function;

  function "-" (x : month_t; k : integer) return month_t is
  begin
    return x + (-k);
  end function;

  function "-" (a, b : month_t) return integer is
  begin
    return month_t'pos(a) - month_t'pos(b);
  end function;

end package body bl_enum_month;

-- Pure and recursive integer and real functions.
library ieee;
use ieee.math_real.all;
use work.bl_types.all;

package bl_math is
  function gcd (a, b : natural) return natural;
  function lcm (a, b : positive) return positive;
  function fib (n : natural) return natural;
  function factorial (n : natural) return positive;
  function ipow (b : integer; e : natural) return integer;
  function isqrt (n : natural) return natural;
  function ilog2 (n : positive) return natural;
  function clog2 (n : positive) return natural;
  function binomial (n, k : natural) return natural;
  function is_prime (n : natural) return boolean;
  function next_prime (n : natural) return positive;
  function collatz_steps (n : positive) return natural;
  function digit_sum (n : natural) return natural;
  function digit_count (n : natural) return positive;
  function ackermann (m, n : natural) return natural;
  function clamp (x, lo, hi : integer) return integer;
  function clamp (x, lo, hi : real) return real;
  function sign_of (x : integer) return integer;
  function sign_of (x : real) return integer;
  function mod_pow (b, e, m : natural) return natural;
  function approx (a, b : real; tol : real := 1.0e-6) return boolean;
  function lerp (a, b, t : real) return real;
  function to_db (ratio : real) return real;
  function triangle (n : natural) return natural;
  function is_power_of_two (n : natural) return boolean;
  function round_up (n : natural; q : positive) return natural;
  function poly_eval (c : real_vec; x : real) return real;
  function newton_sqrt (x : real) return real;
end package bl_math;

package body bl_math is
  function gcd (a, b : natural) return natural is
  begin
    if b = 0 then
      return a;
    end if;
    return gcd(b, a mod b);
  end function;

  function lcm (a, b : positive) return positive is
  begin
    return a / gcd(a, b) * b;
  end function;

  function fib (n : natural) return natural is
  begin
    if n < 2 then
      return n;
    end if;
    return fib(n - 1) + fib(n - 2);
  end function;

  function factorial (n : natural) return positive is
  begin
    if n <= 1 then
      return 1;
    end if;
    return n * factorial(n - 1);
  end function;

  function ipow (b : integer; e : natural) return integer is
    variable h : integer;
  begin
    if e = 0 then
      return 1;
    end if;
    h := ipow(b, e / 2);
    if e mod 2 = 1 then
      return h * h * b;
    end if;
    return h * h;
  end function;

  function isqrt (n : natural) return natural is
    variable r : natural := 0;
  begin
    while (r + 1) * (r + 1) <= n loop
      r := r + 1;
    end loop;
    return r;
  end function;

  function ilog2 (n : positive) return natural is
  begin
    if n = 1 then
      return 0;
    end if;
    return 1 + ilog2(n / 2);
  end function;

  function clog2 (n : positive) return natural is
  begin
    if n = 1 then
      return 0;
    end if;
    return 1 + ilog2(n - 1);
  end function;

  function binomial (n, k : natural) return natural is
  begin
    if k = 0 or k = n then
      return 1;
    elsif k > n then
      return 0;
    end if;
    return binomial(n - 1, k - 1) + binomial(n - 1, k);
  end function;

  function is_prime (n : natural) return boolean is
    variable d : natural := 2;
  begin
    if n < 2 then
      return false;
    end if;
    while d * d <= n loop
      if n mod d = 0 then
        return false;
      end if;
      d := d + 1;
    end loop;
    return true;
  end function;

  function next_prime (n : natural) return positive is
    variable c : positive := n + 1;
  begin
    while not is_prime(c) loop
      c := c + 1;
    end loop;
    return c;
  end function;

  function collatz_steps (n : positive) return natural is
  begin
    if n = 1 then
      return 0;
    elsif n mod 2 = 0 then
      return 1 + collatz_steps(n / 2);
    end if;
    return 1 + collatz_steps(3 * n + 1);
  end function;

  function digit_sum (n : natural) return natural is
  begin
    if n < 10 then
      return n;
    end if;
    return n mod 10 + digit_sum(n / 10);
  end function;

  function digit_count (n : natural) return positive is
  begin
    if n < 10 then
      return 1;
    end if;
    return 1 + digit_count(n / 10);
  end function;

  function ackermann (m, n : natural) return natural is
  begin
    if m = 0 then
      return n + 1;
    elsif n = 0 then
      return ackermann(m - 1, 1);
    end if;
    return ackermann(m - 1, ackermann(m, n - 1));
  end function;

  function clamp (x, lo, hi : integer) return integer is
  begin
    return minimum(maximum(x, lo), hi);
  end function;

  function clamp (x, lo, hi : real) return real is
  begin
    return minimum(maximum(x, lo), hi);
  end function;

  function sign_of (x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    elsif x < 0 then
      return -1;
    end if;
    return 0;
  end function;

  function sign_of (x : real) return integer is
  begin
    if x > 0.0 then
      return 1;
    elsif x < 0.0 then
      return -1;
    end if;
    return 0;
  end function;

  function mod_pow (b, e, m : natural) return natural is
    variable r : natural := 1;
    variable base : natural := b mod m;
    variable ex : natural := e;
  begin
    while ex > 0 loop
      if ex mod 2 = 1 then
        r := (r * base) mod m;
      end if;
      base := (base * base) mod m;
      ex := ex / 2;
    end loop;
    return r;
  end function;

  function approx (a, b : real; tol : real := 1.0e-6) return boolean is
  begin
    return abs (a - b) <= tol;
  end function;

  function lerp (a, b, t : real) return real is
  begin
    return a + (b - a) * t;
  end function;

  function to_db (ratio : real) return real is
  begin
    return 20.0 * log10(ratio);
  end function;

  function triangle (n : natural) return natural is
  begin
    return n * (n + 1) / 2;
  end function;

  function is_power_of_two (n : natural) return boolean is
  begin
    return n > 0 and 2 ** ilog2(n) = n;
  end function;

  function round_up (n : natural; q : positive) return natural is
  begin
    return ((n + q - 1) / q) * q;
  end function;

  function poly_eval (c : real_vec; x : real) return real is
    variable acc : real := 0.0;
  begin
    for i in c'range loop
      acc := acc * x + c(i);
    end loop;
    return acc;
  end function;

  function newton_sqrt (x : real) return real is
    variable g : real := maximum(x, 1.0);
  begin
    for i in 1 to 30 loop
      g := 0.5 * (g + x / g);
    end loop;
    return g;
  end function;
end package body bl_math;

-- Bit manipulation on logic vectors, unsigned values and bit vectors.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bl_types.all;

package bl_bits is
  function popcount (v : std_logic_vector) return natural;
  function popcount (v : unsigned) return natural;
  function popcount (v : bit_vector) return natural;
  function parity (v : std_logic_vector) return std_ulogic;
  function reverse_bits (v : std_logic_vector) return std_logic_vector;
  function reverse_bits (v : unsigned) return unsigned;
  function rotl (v : std_logic_vector; n : natural) return std_logic_vector;
  function rotr (v : std_logic_vector; n : natural) return std_logic_vector;
  function to_gray (v : unsigned) return unsigned;
  function from_gray (v : unsigned) return unsigned;
  function is_onehot (v : std_logic_vector) return boolean;
  function onehot_index (v : std_logic_vector) return integer;
  function to_onehot (i : natural; width : positive) return std_logic_vector;
  function clz (v : unsigned) return natural;
  function ctz (v : unsigned) return natural;
  function msb_index (v : unsigned) return integer;
  function mask_low (n : natural; width : positive) return std_logic_vector;
  function extract (v : std_logic_vector; lo, len : natural) return std_logic_vector;
  function insert (v : std_logic_vector; field : std_logic_vector; lo : natural) return std_logic_vector;
  function any_set (v : std_logic_vector) return boolean;
  function hamming (a, b : std_logic_vector) return natural;
  function interleave (a, b : std_logic_vector) return std_logic_vector;
  function sign_extend (v : std_logic_vector; width : positive) return std_logic_vector;
  function zero_extend (v : std_logic_vector; width : positive) return std_logic_vector;
  function bit_at (v : std_logic_vector; i : natural) return std_ulogic;
  function set_bit (v : std_logic_vector; i : natural; b : std_ulogic := '1') return std_logic_vector;
  function shift_in (v : std_logic_vector; b : std_ulogic) return std_logic_vector;
end package bl_bits;

package body bl_bits is
  function popcount (v : std_logic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function popcount (v : unsigned) return natural is
  begin
    return popcount(std_logic_vector(v));
  end function;

  function popcount (v : bit_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function parity (v : std_logic_vector) return std_ulogic is
  begin
    if popcount(v) mod 2 = 1 then
      return '1';
    end if;
    return '0';
  end function;

  function reverse_bits (v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'reverse_range);
  begin
    for i in v'range loop
      r(i) := v(i);
    end loop;
    return r;
  end function;

  function reverse_bits (v : unsigned) return unsigned is
  begin
    return unsigned(reverse_bits(std_logic_vector(v)));
  end function;

  function rotl (v : std_logic_vector; n : natural) return std_logic_vector is
    constant k : natural := n mod v'length;
    constant x : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    if k = 0 then
      return x;
    end if;
    return x(x'left - k downto 0) & x(x'left downto x'left - k + 1);
  end function;

  function rotr (v : std_logic_vector; n : natural) return std_logic_vector is
  begin
    return rotl(v, v'length - (n mod v'length));
  end function;

  function to_gray (v : unsigned) return unsigned is
  begin
    return v xor shift_right(v, 1);
  end function;

  function from_gray (v : unsigned) return unsigned is
    constant x : unsigned(v'length - 1 downto 0) := v;
    variable r : unsigned(v'length - 1 downto 0) := x;
  begin
    for i in x'left - 1 downto 0 loop
      r(i) := r(i + 1) xor x(i);
    end loop;
    return r;
  end function;

  function is_onehot (v : std_logic_vector) return boolean is
  begin
    return popcount(v) = 1;
  end function;

  function onehot_index (v : std_logic_vector) return integer is
    constant x : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    for i in x'range loop
      if x(i) = '1' then
        return i;
      end if;
    end loop;
    return -1;
  end function;

  function to_onehot (i : natural; width : positive) return std_logic_vector is
    variable r : std_logic_vector(width - 1 downto 0) := (others => '0');
  begin
    r(i) := '1';
    return r;
  end function;

  function clz (v : unsigned) return natural is
    constant x : unsigned(v'length - 1 downto 0) := v;
  begin
    for i in x'range loop
      if x(i) = '1' then
        return x'left - i;
      end if;
    end loop;
    return v'length;
  end function;

  function ctz (v : unsigned) return natural is
    constant x : unsigned(v'length - 1 downto 0) := v;
  begin
    for i in 0 to x'left loop
      if x(i) = '1' then
        return i;
      end if;
    end loop;
    return v'length;
  end function;

  function msb_index (v : unsigned) return integer is
  begin
    return v'length - 1 - clz(v);
  end function;

  function mask_low (n : natural; width : positive) return std_logic_vector is
    variable r : std_logic_vector(width - 1 downto 0) := (others => '0');
  begin
    for i in 0 to minimum(n, width) - 1 loop
      r(i) := '1';
    end loop;
    return r;
  end function;

  function extract (v : std_logic_vector; lo, len : natural) return std_logic_vector is
    constant norm : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    return norm(lo + len - 1 downto lo);
  end function;

  function insert (v : std_logic_vector; field : std_logic_vector; lo : natural) return std_logic_vector is
    variable r : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    r(lo + field'length - 1 downto lo) := field;
    return r;
  end function;

  function any_set (v : std_logic_vector) return boolean is
  begin
    return popcount(v) > 0;
  end function;

  function hamming (a, b : std_logic_vector) return natural is
  begin
    return popcount(a xor b);
  end function;

  function interleave (a, b : std_logic_vector) return std_logic_vector is
    constant ax : std_logic_vector(a'length - 1 downto 0) := a;
    constant bx : std_logic_vector(a'length - 1 downto 0) := b;
    variable r : std_logic_vector(2 * a'length - 1 downto 0);
  begin
    for i in 0 to a'length - 1 loop
      r(2 * i) := bx(i);
      r(2 * i + 1) := ax(i);
    end loop;
    return r;
  end function;

  function sign_extend (v : std_logic_vector; width : positive) return std_logic_vector is
  begin
    return std_logic_vector(resize(signed(v), width));
  end function;

  function zero_extend (v : std_logic_vector; width : positive) return std_logic_vector is
  begin
    return std_logic_vector(resize(unsigned(v), width));
  end function;

  function bit_at (v : std_logic_vector; i : natural) return std_ulogic is
    constant x : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    return x(i);
  end function;

  function set_bit (v : std_logic_vector; i : natural; b : std_ulogic := '1') return std_logic_vector is
    variable r : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    r(i) := b;
    return r;
  end function;

  function shift_in (v : std_logic_vector; b : std_ulogic) return std_logic_vector is
    constant x : std_logic_vector(v'length - 1 downto 0) := v;
  begin
    return x(x'left - 1 downto 0) & b;
  end function;
end package body bl_bits;

-- Checksums and CRC functions with constant tables.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bl_types.all;

package bl_crc is
  type crc_table_t is array (0 to 255) of std_logic_vector(7 downto 0);
  function make_crc8_table (poly : std_logic_vector(7 downto 0)) return crc_table_t;
  function crc8 (data : byte_array; init : std_logic_vector(7 downto 0) := x"00") return std_logic_vector;
  function crc8 (s : string) return std_logic_vector;
  function crc16_step (crc : std_logic_vector(15 downto 0); b : std_logic_vector(7 downto 0)) return std_logic_vector;
  function crc16 (data : byte_array) return std_logic_vector;
  function adler32 (s : string) return natural;
  function fcheck16 (data : byte_array) return natural;
end package bl_crc;

package body bl_crc is
  function make_crc8_table (poly : std_logic_vector(7 downto 0)) return crc_table_t is
    variable t : crc_table_t;
    variable c : std_logic_vector(7 downto 0);
  begin
    for i in 0 to 255 loop
      c := std_logic_vector(to_unsigned(i, 8));
      for k in 1 to 8 loop
        if c(7) = '1' then
          c := (c(6 downto 0) & '0') xor poly;
        else
          c := c(6 downto 0) & '0';
        end if;
      end loop;
      t(i) := c;
    end loop;
    return t;
  end function;

  constant crc8_table : crc_table_t := make_crc8_table(x"07");

  function crc8 (data : byte_array; init : std_logic_vector(7 downto 0) := x"00") return std_logic_vector is
    variable c : std_logic_vector(7 downto 0) := init;
  begin
    for i in data'range loop
      c := crc8_table(to_integer(unsigned(c xor data(i))));
    end loop;
    return c;
  end function;

  function crc8 (s : string) return std_logic_vector is
    variable c : std_logic_vector(7 downto 0) := x"00";
  begin
    for i in s'range loop
      c := crc8_table(to_integer(unsigned(c xor std_logic_vector(to_unsigned(character'pos(s(i)), 8)))));
    end loop;
    return c;
  end function;

  function crc16_step (crc : std_logic_vector(15 downto 0); b : std_logic_vector(7 downto 0)) return std_logic_vector is
    variable c : std_logic_vector(15 downto 0) := crc xor (b & x"00");
  begin
    for k in 1 to 8 loop
      if c(15) = '1' then
        c := (c(14 downto 0) & '0') xor x"1021";
      else
        c := c(14 downto 0) & '0';
      end if;
    end loop;
    return c;
  end function;

  function crc16 (data : byte_array) return std_logic_vector is
    variable c : std_logic_vector(15 downto 0) := x"FFFF";
  begin
    for i in data'range loop
      c := crc16_step(c, data(i));
    end loop;
    return c;
  end function;

  function adler32 (s : string) return natural is
    variable a : natural := 1;
    variable b : natural := 0;
  begin
    for i in s'range loop
      a := (a + character'pos(s(i))) mod 65521;
      b := (b + a) mod 65521;
    end loop;
    return b * 65536 + a;
  end function;

  function fcheck16 (data : byte_array) return natural is
    variable s1, s2 : natural := 0;
  begin
    for i in data'range loop
      s1 := (s1 + to_integer(unsigned(data(i)))) mod 255;
      s2 := (s2 + s1) mod 255;
    end loop;
    return s2 * 256 + s1;
  end function;

end package body bl_crc;

-- Calendar helpers built on the month and weekday enumerations.
use work.bl_enum_month.all;
use work.bl_enum_weekday.all;

package bl_calendar is
  type date_t is record
    year : positive;
    month : month_t;
    day : positive range 1 to 31;
  end record;
  function is_leap (y : positive) return boolean;
  function days_in (y : positive; m : month_t) return positive;
  function day_of_year (d : date_t) return positive;
  function valid (d : date_t) return boolean;
  function weekday_of (d : date_t) return weekday_t;
  function next_day (d : date_t) return date_t;
  function add_days (d : date_t; n : natural) return date_t;
  function days_between (a, b : date_t) return natural;
  function is_weekend (w : weekday_t) return boolean;
  function to_string (d : date_t) return string;
end package bl_calendar;

package body bl_calendar is
  function is_leap (y : positive) return boolean is
  begin
    return (y mod 4 = 0 and y mod 100 /= 0) or y mod 400 = 0;
  end function;

  function days_in (y : positive; m : month_t) return positive is
  begin
    case m is
      when apr | jun | sep | nov => return 30;
      when feb =>
        if is_leap(y) then
          return 29;
        end if;
        return 28;
      when others => return 31;
    end case;
  end function;

  function day_of_year (d : date_t) return positive is
    variable n : natural := d.day;
  begin
    for m in month_t loop
      exit when m = d.month;
      n := n + days_in(d.year, m);
    end loop;
    return n;
  end function;

  function valid (d : date_t) return boolean is
  begin
    return d.day <= days_in(d.year, d.month);
  end function;

  function weekday_of (d : date_t) return weekday_t is
    variable days : natural := 0;
  begin
    for y in 1970 to d.year - 1 loop
      if is_leap(y) then
        days := days + 366;
      else
        days := days + 365;
      end if;
    end loop;
    days := days + day_of_year(d) - 1;
    return weekday_t'val((days + 3) mod 7);
  end function;

  function next_day (d : date_t) return date_t is
  begin
    if d.day < days_in(d.year, d.month) then
      return (d.year, d.month, d.day + 1);
    elsif d.month = dec then
      return (d.year + 1, jan, 1);
    end if;
    return (d.year, month_t'succ(d.month), 1);
  end function;

  function add_days (d : date_t; n : natural) return date_t is
    variable r : date_t := d;
  begin
    for i in 1 to n loop
      r := next_day(r);
    end loop;
    return r;
  end function;

  function days_between (a, b : date_t) return natural is
    variable r : date_t := a;
    variable n : natural := 0;
  begin
    while r.year /= b.year or r.month /= b.month or r.day /= b.day loop
      r := next_day(r);
      n := n + 1;
    end loop;
    return n;
  end function;

  function is_weekend (w : weekday_t) return boolean is
  begin
    return w = saturday or w = sunday;
  end function;

  function to_string (d : date_t) return string is
  begin
    return integer'image(d.year) & "-" & to_string(d.month) & "-" & integer'image(d.day);
  end function;
end package body bl_calendar;

-- Text output and parsing helpers on lines and text files.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
use work.bl_types.all;
use work.bl_chars.all;
use work.bl_conv.all;
use work.bl_strings.all;

package bl_text is
  procedure put (l : inout line; s : string);
  procedure put (l : inout line; v : integer);
  procedure put (l : inout line; v : std_logic_vector);
  procedure put (l : inout line; v : boolean);
  procedure put (l : inout line; v : real);
  procedure put (l : inout line; v : time);
  procedure put_hex (l : inout line; v : std_logic_vector);
  procedure put_padded (l : inout line; s : string; w : natural);
  procedure emit (file f : text; l : inout line);
  procedure emit (file f : text; s : string);
  procedure emit_kv (file f : text; key : string; value : integer);
  procedure emit_table (file f : text; v : int_vec; per_line : positive := 8);
  procedure skip_spaces (l : inout line);
  procedure next_word (l : inout line; w : out string; n : out natural);
  procedure next_int (l : inout line; v : out integer; ok : out boolean);
  procedure next_hex (l : inout line; v : out std_logic_vector; ok : out boolean);
  impure function line_of (s : string) return line;
  impure function words_in (s : string) return natural;
  impure function sum_of_ints (s : string) return integer;
  procedure drop (l : inout line);
end package bl_text;

package body bl_text is
  procedure put (l : inout line; s : string) is
  begin
    write(l, s);
  end procedure;

  procedure put (l : inout line; v : integer) is
  begin
    write(l, v);
  end procedure;

  procedure put (l : inout line; v : std_logic_vector) is
  begin
    write(l, to_str(v));
  end procedure;

  procedure put (l : inout line; v : boolean) is
  begin
    write(l, v);
  end procedure;

  procedure put (l : inout line; v : real) is
  begin
    write(l, to_str(v, 3));
  end procedure;

  procedure put (l : inout line; v : time) is
  begin
    write(l, v, right, 0, ns);
  end procedure;

  procedure put_hex (l : inout line; v : std_logic_vector) is
  begin
    write(l, "0x" & to_hex(v));
  end procedure;

  procedure put_padded (l : inout line; s : string; w : natural) is
  begin
    write(l, s);
    for i in s'length + 1 to w loop
      write(l, ' ');
    end loop;
  end procedure;

  procedure emit (file f : text; l : inout line) is
  begin
    writeline(f, l);
  end procedure;

  procedure emit (file f : text; s : string) is
    variable l : line;
  begin
    write(l, s);
    writeline(f, l);
  end procedure;

  procedure emit_kv (file f : text; key : string; value : integer) is
    variable l : line;
  begin
    put_padded(l, key, 10);
    write(l, string'("= "));
    write(l, value);
    writeline(f, l);
  end procedure;

  procedure emit_table (file f : text; v : int_vec; per_line : positive := 8) is
    variable l : line;
    variable n : natural := 0;
  begin
    for i in v'range loop
      write(l, v(i), right, 5);
      n := n + 1;
      if n mod per_line = 0 or i = v'right then
        writeline(f, l);
      end if;
    end loop;
  end procedure;

  procedure skip_spaces (l : inout line) is
    variable c : character;
    variable good : boolean;
  begin
    while l /= null and l'length > 0 loop
      if is_space(l(l'left)) then
        read(l, c, good);
      else
        exit;
      end if;
    end loop;
  end procedure;

  procedure next_word (l : inout line; w : out string; n : out natural) is
    variable c : character;
    variable good : boolean;
    variable k : natural := 0;
    variable tmp : string(1 to w'length) := (others => ' ');
  begin
    skip_spaces(l);
    while l /= null and l'length > 0 and not is_space(l(l'left)) loop
      read(l, c, good);
      if k < w'length then
        k := k + 1;
        tmp(k) := c;
      end if;
    end loop;
    w := tmp;
    n := k;
  end procedure;

  procedure next_int (l : inout line; v : out integer; ok : out boolean) is
    variable tmp : string(1 to 16);
    variable n : natural;
  begin
    next_word(l, tmp, n);
    if n > 0 and is_number(tmp(1 to n)) then
      v := to_int(tmp(1 to n));
      ok := true;
    else
      v := 0;
      ok := false;
    end if;
  end procedure;

  procedure next_hex (l : inout line; v : out std_logic_vector; ok : out boolean) is
    variable tmp : string(1 to 16);
    variable n : natural;
    variable good : boolean := true;
  begin
    next_word(l, tmp, n);
    for i in 1 to n loop
      good := good and is_hex(tmp(i));
    end loop;
    if good and n > 0 and 4 * n <= v'length then
      v := std_logic_vector(resize(unsigned(parse_hex(tmp(1 to n))), v'length));
      ok := true;
    else
      v := (v'range => '0');
      ok := false;
    end if;
  end procedure;

  impure function line_of (s : string) return line is
    variable l : line;
  begin
    write(l, s);
    return l;
  end function;

  impure function words_in (s : string) return natural is
    variable l : line := new string'(s);
    variable w : string(1 to 20);
    variable n, count : natural := 0;
  begin
    loop
      next_word(l, w, n);
      exit when n = 0;
      count := count + 1;
    end loop;
    deallocate(l);
    return count;
  end function;

  impure function sum_of_ints (s : string) return integer is
    variable l : line := new string'(s);
    variable v, total : integer := 0;
    variable ok : boolean;
  begin
    loop
      next_int(l, v, ok);
      exit when not ok;
      total := total + v;
    end loop;
    deallocate(l);
    return total;
  end function;

  procedure drop (l : inout line) is
  begin
    if l /= null then
      deallocate(l);
    end if;
  end procedure;
end package body bl_text;

-- Access types: linked lists, binary trees and string pointers.
use work.bl_types.all;

package bl_linked is
  type node_t;
  type node_ptr is access node_t;
  type node_t is record
    value : integer;
    next_node : node_ptr;
  end record;
  type tree_t;
  type tree_ptr is access tree_t;
  type tree_t is record
    key : integer;
    left, right : tree_ptr;
  end record;
  type str_node_t;
  type str_node_ptr is access str_node_t;
  type str_node_t is record
    text : str_ptr;
    next_node : str_node_ptr;
  end record;

  procedure push_front (head : inout node_ptr; v : integer);
  procedure append (head : inout node_ptr; v : integer);
  procedure free_list (head : inout node_ptr);
  procedure reverse_list (head : inout node_ptr);
  procedure remove_value (head : inout node_ptr; v : integer);
  procedure length (variable head : in node_ptr; result : out natural);
  procedure sum (variable head : in node_ptr; result : out integer);
  procedure nth (variable head : in node_ptr; n : natural; result : out integer);
  procedure find (variable head : in node_ptr; v : integer; result : out boolean);
  procedure to_vec (variable head : in node_ptr; result : out int_vec);
  procedure tree_insert (root : inout tree_ptr; k : integer);
  procedure tree_contains (variable root : in tree_ptr; k : integer; result : out boolean);
  procedure tree_height (variable root : in tree_ptr; result : out natural);
  procedure tree_size (variable root : in tree_ptr; result : out natural);
  procedure tree_min (variable root : in tree_ptr; result : out integer);
  procedure tree_max (variable root : in tree_ptr; result : out integer);
  procedure tree_flatten (variable root : in tree_ptr; v : inout int_vec; n : inout natural);
  procedure tree_free (root : inout tree_ptr);
  impure function dup (s : string) return str_ptr;
  procedure free_str (p : inout str_ptr);
  procedure add_text (head : inout str_node_ptr; s : string);
  procedure text_count (variable head : in str_node_ptr; result : out natural);
  procedure text_at (variable head : in str_node_ptr; n : natural; result : out str_ptr);
  procedure free_texts (head : inout str_node_ptr);
end package bl_linked;

package body bl_linked is
  procedure push_front (head : inout node_ptr; v : integer) is
  begin
    head := new node_t'(value => v, next_node => head);
  end procedure;

  procedure append (head : inout node_ptr; v : integer) is
    variable p : node_ptr := head;
  begin
    if head = null then
      head := new node_t'(v, null);
      return;
    end if;
    while p.next_node /= null loop
      p := p.next_node;
    end loop;
    p.next_node := new node_t'(v, null);
  end procedure;

  procedure free_list (head : inout node_ptr) is
    variable p : node_ptr;
  begin
    while head /= null loop
      p := head.next_node;
      deallocate(head);
      head := p;
    end loop;
  end procedure;

  procedure reverse_list (head : inout node_ptr) is
    variable prev, cur, nxt : node_ptr;
  begin
    cur := head;
    while cur /= null loop
      nxt := cur.next_node;
      cur.next_node := prev;
      prev := cur;
      cur := nxt;
    end loop;
    head := prev;
  end procedure;

  procedure remove_value (head : inout node_ptr; v : integer) is
    variable p, dead : node_ptr;
  begin
    if head = null then
      return;
    end if;
    if head.value = v then
      dead := head;
      head := head.next_node;
      deallocate(dead);
      return;
    end if;
    p := head;
    while p.next_node /= null loop
      if p.next_node.value = v then
        dead := p.next_node;
        p.next_node := dead.next_node;
        deallocate(dead);
        return;
      end if;
      p := p.next_node;
    end loop;
  end procedure;

  procedure length (variable head : in node_ptr; result : out natural) is
    variable p : node_ptr := head;
    variable n : natural := 0;
  begin
    while p /= null loop
      n := n + 1;
      p := p.next_node;
    end loop;
    result := n;
  end procedure;

  procedure sum (variable head : in node_ptr; result : out integer) is
    variable p : node_ptr := head;
    variable s : integer := 0;
  begin
    while p /= null loop
      s := s + p.value;
      p := p.next_node;
    end loop;
    result := s;
  end procedure;

  procedure nth (variable head : in node_ptr; n : natural; result : out integer) is
    variable p : node_ptr := head;
  begin
    for i in 1 to n loop
      p := p.next_node;
    end loop;
    result := p.value;
  end procedure;

  procedure find (variable head : in node_ptr; v : integer; result : out boolean) is
    variable p : node_ptr := head;
  begin
    result := false;
    while p /= null loop
      if p.value = v then
        result := true;
        return;
      end if;
      p := p.next_node;
    end loop;
  end procedure;

  procedure to_vec (variable head : in node_ptr; result : out int_vec) is
    variable p : node_ptr := head;
  begin
    for i in result'range loop
      result(i) := p.value;
      p := p.next_node;
    end loop;
  end procedure;

  procedure tree_insert (root : inout tree_ptr; k : integer) is
  begin
    if root = null then
      root := new tree_t'(k, null, null);
    elsif k < root.key then
      tree_insert(root.left, k);
    elsif k > root.key then
      tree_insert(root.right, k);
    end if;
  end procedure;

  procedure tree_contains (variable root : in tree_ptr; k : integer; result : out boolean) is
    variable sub : tree_ptr;
  begin
    if root = null then
      result := false;
    elsif k = root.key then
      result := true;
    else
      if k < root.key then
        sub := root.left;
      else
        sub := root.right;
      end if;
      tree_contains(sub, k, result);
    end if;
  end procedure;

  procedure tree_height (variable root : in tree_ptr; result : out natural) is
    variable l, r : natural := 0;
    variable sub : tree_ptr;
  begin
    if root = null then
      result := 0;
      return;
    end if;
    sub := root.left;
    tree_height(sub, l);
    sub := root.right;
    tree_height(sub, r);
    result := 1 + maximum(l, r);
  end procedure;

  procedure tree_size (variable root : in tree_ptr; result : out natural) is
    variable l, r : natural := 0;
    variable sub : tree_ptr;
  begin
    if root = null then
      result := 0;
      return;
    end if;
    sub := root.left;
    tree_size(sub, l);
    sub := root.right;
    tree_size(sub, r);
    result := 1 + l + r;
  end procedure;

  procedure tree_min (variable root : in tree_ptr; result : out integer) is
    variable sub : tree_ptr := root;
  begin
    while sub.left /= null loop
      sub := sub.left;
    end loop;
    result := sub.key;
  end procedure;

  procedure tree_max (variable root : in tree_ptr; result : out integer) is
    variable sub : tree_ptr := root;
  begin
    while sub.right /= null loop
      sub := sub.right;
    end loop;
    result := sub.key;
  end procedure;

  procedure tree_flatten (variable root : in tree_ptr; v : inout int_vec; n : inout natural) is
    variable sub : tree_ptr;
  begin
    if root /= null then
      sub := root.left;
      tree_flatten(sub, v, n);
      v(v'left + n) := root.key;
      n := n + 1;
      sub := root.right;
      tree_flatten(sub, v, n);
    end if;
  end procedure;

  procedure tree_free (root : inout tree_ptr) is
  begin
    if root /= null then
      tree_free(root.left);
      tree_free(root.right);
      deallocate(root);
    end if;
  end procedure;

  impure function dup (s : string) return str_ptr is
  begin
    return new string'(s);
  end function;

  procedure free_str (p : inout str_ptr) is
  begin
    if p /= null then
      deallocate(p);
    end if;
  end procedure;

  procedure add_text (head : inout str_node_ptr; s : string) is
    variable p : str_node_ptr;
  begin
    if head = null then
      head := new str_node_t'(dup(s), null);
      return;
    end if;
    p := head;
    while p.next_node /= null loop
      p := p.next_node;
    end loop;
    p.next_node := new str_node_t'(dup(s), null);
  end procedure;

  procedure text_count (variable head : in str_node_ptr; result : out natural) is
    variable p : str_node_ptr := head;
    variable n : natural := 0;
  begin
    while p /= null loop
      n := n + 1;
      p := p.next_node;
    end loop;
    result := n;
  end procedure;

  procedure text_at (variable head : in str_node_ptr; n : natural; result : out str_ptr) is
    variable p : str_node_ptr := head;
  begin
    for i in 1 to n loop
      p := p.next_node;
    end loop;
    result := p.text;
  end procedure;

  procedure free_texts (head : inout str_node_ptr) is
    variable p : str_node_ptr;
  begin
    while head /= null loop
      p := head.next_node;
      free_str(head.text);
      deallocate(head);
      head := p;
    end loop;
  end procedure;
end package body bl_linked;

-- Protected types: queue, stack, hash table, random generator, logger, counter bank.
library ieee;
use ieee.math_real.all;
use work.bl_types.all;
use work.bl_strings.all;

package bl_prot is
  type int_queue_t is protected
    procedure push (v : integer);
    impure function pop return integer;
    impure function peek return integer;
    impure function size return natural;
    impure function is_empty return boolean;
    impure function is_full return boolean;
    procedure clear;
    impure function total_pushed return natural;
  end protected int_queue_t;

  type int_stack_t is protected
    procedure push (v : integer);
    impure function pop return integer;
    impure function top return integer;
    impure function depth return natural;
    procedure clear;
    impure function snapshot return int_vec;
  end protected int_stack_t;

  type int_map_t is protected
    procedure put (key : integer; value : integer);
    impure function get (key : integer; fallback : integer := -1) return integer;
    impure function has (key : integer) return boolean;
    procedure remove (key : integer);
    impure function size return natural;
    procedure clear;
  end protected int_map_t;

  type string_pool_t is protected
    impure function add (s : string) return natural;
    impure function find (s : string) return integer;
    impure function get (id : natural) return string;
    impure function count return natural;
    impure function intern (s : string) return natural;
  end protected string_pool_t;

  type rng_t is protected
    procedure seed (s : natural);
    impure function next_raw return natural;
    impure function uniform (lo, hi : integer) return integer;
    impure function coin return boolean;
    impure function real01 return real;
    impure function pick (v : int_vec) return integer;
    impure function bits (n : positive range 1 to 20) return natural;
    procedure shuffle (v : inout int_vec);
  end protected rng_t;

  type logger_t is protected
    procedure set_threshold (l : severity_level);
    procedure log (l : severity_level; msg : string);
    impure function count (l : severity_level) return natural;
    impure function total return natural;
    impure function last_message return string;
    procedure reset;
  end protected logger_t;

  type counter_bank_t is protected
    procedure bump (name : string; by : natural := 1);
    impure function value (name : string) return natural;
    impure function names return natural;
    impure function sum return natural;
  end protected counter_bank_t;

  type stats_t is protected
    procedure add (x : real);
    impure function n return natural;
    impure function mean return real;
    impure function stddev return real;
    impure function min_value return real;
    impure function max_value return real;
    procedure reset;
  end protected stats_t;
end package bl_prot;

package body bl_prot is
  type int_queue_t is protected body
    constant cap : positive := 64;
    variable buf : int_vec(0 to cap - 1);
    variable rd, wr, cnt, pushed : natural := 0;

    procedure push (v : integer) is
    begin
      assert cnt < cap report "queue overflow" severity failure;
      buf(wr) := v;
      wr := (wr + 1) mod cap;
      cnt := cnt + 1;
      pushed := pushed + 1;
    end procedure;

    impure function pop return integer is
      variable v : integer;
    begin
      assert cnt > 0 report "queue underflow" severity failure;
      v := buf(rd);
      rd := (rd + 1) mod cap;
      cnt := cnt - 1;
      return v;
    end function;

    impure function peek return integer is
    begin
      return buf(rd);
    end function;

    impure function size return natural is
    begin
      return cnt;
    end function;

    impure function is_empty return boolean is
    begin
      return cnt = 0;
    end function;

    impure function is_full return boolean is
    begin
      return cnt = cap;
    end function;

    procedure clear is
    begin
      rd := 0;
      wr := 0;
      cnt := 0;
    end procedure;

    impure function total_pushed return natural is
    begin
      return pushed;
    end function;
  end protected body int_queue_t;

  type int_stack_t is protected body
    variable items : int_vec(0 to 255);
    variable sp : natural := 0;

    procedure push (v : integer) is
    begin
      items(sp) := v;
      sp := sp + 1;
    end procedure;

    impure function pop return integer is
    begin
      sp := sp - 1;
      return items(sp);
    end function;

    impure function top return integer is
    begin
      return items(sp - 1);
    end function;

    impure function depth return natural is
    begin
      return sp;
    end function;

    procedure clear is
    begin
      sp := 0;
    end procedure;

    impure function snapshot return int_vec is
    begin
      return items(0 to sp - 1);
    end function;
  end protected body int_stack_t;

  type int_map_t is protected body
    constant slots : positive := 128;
    type state_t is (empty, used, deleted);
    type state_arr is array (0 to slots - 1) of state_t;
    variable st : state_arr := (others => empty);
    variable keys, vals : int_vec(0 to slots - 1) := (others => 0);
    variable used_count : natural := 0;

    impure function locate (key : integer) return integer is
      variable i : natural := (abs key * 37) mod slots;
    begin
      for probe in 1 to slots loop
        if st(i) = empty then
          return -1;
        elsif st(i) = used and keys(i) = key then
          return i;
        end if;
        i := (i + 1) mod slots;
      end loop;
      return -1;
    end function;

    procedure put (key : integer; value : integer) is
      variable i : integer := locate(key);
    begin
      if i >= 0 then
        vals(i) := value;
        return;
      end if;
      i := (abs key * 37) mod slots;
      while st(i) = used loop
        i := (i + 1) mod slots;
      end loop;
      st(i) := used;
      keys(i) := key;
      vals(i) := value;
      used_count := used_count + 1;
    end procedure;

    impure function get (key : integer; fallback : integer := -1) return integer is
      constant i : integer := locate(key);
    begin
      if i < 0 then
        return fallback;
      end if;
      return vals(i);
    end function;

    impure function has (key : integer) return boolean is
    begin
      return locate(key) >= 0;
    end function;

    procedure remove (key : integer) is
      constant i : integer := locate(key);
    begin
      if i >= 0 then
        st(i) := deleted;
        used_count := used_count - 1;
      end if;
    end procedure;

    impure function size return natural is
    begin
      return used_count;
    end function;

    procedure clear is
    begin
      st := (others => empty);
      used_count := 0;
    end procedure;
  end protected body int_map_t;

  type string_pool_t is protected body
    variable table : string_vec(0 to 63)(1 to 24);
    variable lens : int_vec(0 to 63);
    variable n_items : natural := 0;

    impure function add (s : string) return natural is
      variable padded : string(1 to 24) := (others => ' ');
    begin
      padded(1 to minimum(s'length, 24)) := s(s'left to s'left + minimum(s'length, 24) - 1);
      table(n_items) := padded;
      lens(n_items) := minimum(s'length, 24);
      n_items := n_items + 1;
      return n_items - 1;
    end function;

    impure function find (s : string) return integer is
    begin
      for i in 0 to n_items - 1 loop
        if table(i)(1 to lens(i)) = s then
          return i;
        end if;
      end loop;
      return -1;
    end function;

    impure function get (id : natural) return string is
    begin
      return table(id)(1 to lens(id));
    end function;

    impure function count return natural is
    begin
      return n_items;
    end function;

    impure function intern (s : string) return natural is
      constant k : integer := find(s);
    begin
      if k >= 0 then
        return k;
      end if;
      return add(s);
    end function;
  end protected body string_pool_t;

  type rng_t is protected body
    constant modulus : positive := 2097152;
    variable state : natural := 12345;

    procedure seed (s : natural) is
    begin
      state := s mod modulus;
    end procedure;

    impure function next_raw return natural is
    begin
      state := (state * 1021 + 12345) mod modulus;
      return state;
    end function;

    impure function uniform (lo, hi : integer) return integer is
    begin
      return lo + ((next_raw / 8) mod (hi - lo + 1));
    end function;

    impure function coin return boolean is
    begin
      return (next_raw / 4096) mod 2 = 1;
    end function;

    impure function real01 return real is
    begin
      return real(next_raw) / real(modulus);
    end function;

    impure function pick (v : int_vec) return integer is
    begin
      return v(v'left + next_raw mod v'length);
    end function;

    impure function bits (n : positive range 1 to 20) return natural is
    begin
      return next_raw mod (2 ** n);
    end function;

    procedure shuffle (v : inout int_vec) is
      variable j : natural;
      variable t : integer;
    begin
      for i in v'right downto v'left + 1 loop
        j := v'left + next_raw mod (i - v'left + 1);
        t := v(i);
        v(i) := v(j);
        v(j) := t;
      end loop;
    end procedure;
  end protected body rng_t;

  type logger_t is protected body
    variable threshold : severity_level := note;
    variable counts : sev_count_t := (others => 0);
    variable last : string(1 to 40) := (others => ' ');
    variable last_len : natural := 0;

    procedure set_threshold (l : severity_level) is
    begin
      threshold := l;
    end procedure;

    procedure log (l : severity_level; msg : string) is
    begin
      if l >= threshold then
        counts(l) := counts(l) + 1;
        last_len := minimum(msg'length, 40);
        last(1 to last_len) := msg(msg'left to msg'left + last_len - 1);
      end if;
    end procedure;

    impure function count (l : severity_level) return natural is
    begin
      return counts(l);
    end function;

    impure function total return natural is
      variable t : natural := 0;
    begin
      for l in severity_level loop
        t := t + counts(l);
      end loop;
      return t;
    end function;

    impure function last_message return string is
    begin
      return last(1 to last_len);
    end function;

    procedure reset is
    begin
      counts := (others => 0);
      last_len := 0;
    end procedure;
  end protected body logger_t;

  type counter_bank_t is protected body
    variable keys : string_vec(0 to 15)(1 to 12);
    variable klen : int_vec(0 to 15) := (others => 0);
    variable vals : int_vec(0 to 15) := (others => 0);
    variable used : natural := 0;

    impure function slot (name : string) return integer is
    begin
      for i in 0 to used - 1 loop
        if keys(i)(1 to klen(i)) = name then
          return i;
        end if;
      end loop;
      return -1;
    end function;

    procedure bump (name : string; by : natural := 1) is
      variable i : integer := slot(name);
    begin
      if i < 0 then
        i := used;
        used := used + 1;
        keys(i)(1 to name'length) := name;
        klen(i) := name'length;
      end if;
      vals(i) := vals(i) + by;
    end procedure;

    impure function value (name : string) return natural is
      constant i : integer := slot(name);
    begin
      if i < 0 then
        return 0;
      end if;
      return vals(i);
    end function;

    impure function names return natural is
    begin
      return used;
    end function;

    impure function sum return natural is
      variable s : natural := 0;
    begin
      for i in 0 to used - 1 loop
        s := s + vals(i);
      end loop;
      return s;
    end function;
  end protected body counter_bank_t;

  type stats_t is protected body
    variable cnt : natural := 0;
    variable acc, acc2 : real := 0.0;
    variable lo : real := real'high;
    variable hi : real := real'low;

    procedure add (x : real) is
    begin
      cnt := cnt + 1;
      acc := acc + x;
      acc2 := acc2 + x * x;
      lo := minimum(lo, x);
      hi := maximum(hi, x);
    end procedure;

    impure function n return natural is
    begin
      return cnt;
    end function;

    impure function mean return real is
    begin
      return acc / real(cnt);
    end function;

    impure function stddev return real is
      constant m : real := acc / real(cnt);
    begin
      return sqrt(maximum(acc2 / real(cnt) - m * m, 0.0));
    end function;

    impure function min_value return real is
    begin
      return lo;
    end function;

    impure function max_value return real is
    begin
      return hi;
    end function;

    procedure reset is
    begin
      cnt := 0;
      acc := 0.0;
      acc2 := 0.0;
      lo := real'high;
      hi := real'low;
    end procedure;
  end protected body stats_t;
end package body bl_prot;

-- Generic package: bounded stack of any element type with an image function.
package bl_gen_stack is
  generic (
    type elem;
    capacity : positive := 16;
    function img (e : elem) return string
  );
  type elem_arr is array (natural range <>) of elem;
  type stack_t is record
    items : elem_arr(0 to capacity - 1);
    sp : natural;
  end record;
  procedure push (s : inout stack_t; e : elem);
  procedure pop (s : inout stack_t; e : out elem);
  function top (s : stack_t) return elem;
  function depth (s : stack_t) return natural;
  function is_full (s : stack_t) return boolean;
  function describe (s : stack_t) return string;
end package bl_gen_stack;

package body bl_gen_stack is
  procedure push (s : inout stack_t; e : elem) is
  begin
    assert s.sp < capacity report "generic stack overflow" severity failure;
    s.items(s.sp) := e;
    s.sp := s.sp + 1;
  end procedure;

  procedure pop (s : inout stack_t; e : out elem) is
  begin
    s.sp := s.sp - 1;
    e := s.items(s.sp);
  end procedure;

  function top (s : stack_t) return elem is
  begin
    return s.items(s.sp - 1);
  end function;

  function depth (s : stack_t) return natural is
  begin
    return s.sp;
  end function;

  function is_full (s : stack_t) return boolean is
  begin
    return s.sp = capacity;
  end function;

  function describe (s : stack_t) return string is
  begin
    if s.sp = 0 then
      return "<>";
    elsif s.sp = 1 then
      return "<" & img(s.items(0)) & ">";
    end if;
    return "<" & img(s.items(0)) & ".." & img(s.items(s.sp - 1)) & ">";
  end function;
end package body bl_gen_stack;

-- Generic package: minimum, maximum, clamp and insertion sort over an ordered type.
package bl_gen_minmax is
  generic (
    type t;
    function "<" (a, b : t) return boolean is <>
  );
  type arr is array (natural range <>) of t;
  function min2 (a, b : t) return t;
  function max2 (a, b : t) return t;
  function clamp (x, lo, hi : t) return t;
  function min_of (v : arr) return t;
  function max_of (v : arr) return t;
  function is_sorted (v : arr) return boolean;
  procedure sort (v : inout arr);
  function sorted_copy (v : arr) return arr;
  function index_of_min (v : arr) return natural;
end package bl_gen_minmax;

package body bl_gen_minmax is
  function min2 (a, b : t) return t is
  begin
    if b < a then
      return b;
    end if;
    return a;
  end function;

  function max2 (a, b : t) return t is
  begin
    if a < b then
      return b;
    end if;
    return a;
  end function;

  function clamp (x, lo, hi : t) return t is
  begin
    return min2(max2(x, lo), hi);
  end function;

  function min_of (v : arr) return t is
    variable m : t := v(v'left);
  begin
    for i in v'range loop
      m := min2(m, v(i));
    end loop;
    return m;
  end function;

  function max_of (v : arr) return t is
    variable m : t := v(v'left);
  begin
    for i in v'range loop
      m := max2(m, v(i));
    end loop;
    return m;
  end function;

  function is_sorted (v : arr) return boolean is
  begin
    for i in v'left to v'right - 1 loop
      if v(i + 1) < v(i) then
        return false;
      end if;
    end loop;
    return true;
  end function;

  procedure sort (v : inout arr) is
    variable key : t;
    variable j : integer;
  begin
    for i in v'left + 1 to v'right loop
      key := v(i);
      j := i - 1;
      while j >= v'left and key < v(j) loop
        v(j + 1) := v(j);
        j := j - 1;
      end loop;
      v(j + 1) := key;
    end loop;
  end procedure;

  function sorted_copy (v : arr) return arr is
    variable c : arr(v'range) := v;
  begin
    sort(c);
    return c;
  end function;

  function index_of_min (v : arr) return natural is
    variable k : natural := v'left;
  begin
    for i in v'range loop
      if v(i) < v(k) then
        k := i;
      end if;
    end loop;
    return k;
  end function;
end package body bl_gen_minmax;

-- Generic package: a pair of two types.
package bl_gen_pair is
  generic (type a_t; type b_t);
  type pair_t is record
    first : a_t;
    second : b_t;
  end record;
  type pair_arr is array (natural range <>) of pair_t;
  function make (a : a_t; b : b_t) return pair_t;
end package bl_gen_pair;

package body bl_gen_pair is
  function make (a : a_t; b : b_t) return pair_t is
  begin
    return (first => a, second => b);
  end function;
end package body bl_gen_pair;

-- Generic package taking an instance of the pair package: association list lookup.
package bl_gen_assoc is
  generic (
    package pairs is new work.bl_gen_pair generic map (<>);
    function same (x, y : pairs.a_t) return boolean
  );
  function lookup (tab : pairs.pair_arr; key : pairs.a_t; fallback : pairs.b_t) return pairs.b_t;
  function has_key (tab : pairs.pair_arr; key : pairs.a_t) return boolean;
  function key_count (tab : pairs.pair_arr; key : pairs.a_t) return natural;
end package bl_gen_assoc;

package body bl_gen_assoc is
  function lookup (tab : pairs.pair_arr; key : pairs.a_t; fallback : pairs.b_t) return pairs.b_t is
  begin
    for i in tab'range loop
      if same(tab(i).first, key) then
        return tab(i).second;
      end if;
    end loop;
    return fallback;
  end function;

  function has_key (tab : pairs.pair_arr; key : pairs.a_t) return boolean is
  begin
    for i in tab'range loop
      if same(tab(i).first, key) then
        return true;
      end if;
    end loop;
    return false;
  end function;

  function key_count (tab : pairs.pair_arr; key : pairs.a_t) return natural is
    variable n : natural := 0;
  begin
    for i in tab'range loop
      if same(tab(i).first, key) then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
end package body bl_gen_assoc;

-- Generic package: fixed-size delay line with a constant initial value.
package bl_gen_delay is
  generic (type t; depth : positive; init : t);
  type line_t is array (0 to depth - 1) of t;
  constant empty_line : line_t := (others => init);
  function shift (l : line_t; x : t) return line_t;
  function oldest (l : line_t) return t;
  function newest (l : line_t) return t;
end package bl_gen_delay;

package body bl_gen_delay is
  function shift (l : line_t; x : t) return line_t is
    variable r : line_t;
  begin
    r(0) := x;
    for i in 1 to depth - 1 loop
      r(i) := l(i - 1);
    end loop;
    return r;
  end function;

  function oldest (l : line_t) return t is
  begin
    return l(depth - 1);
  end function;

  function newest (l : line_t) return t is
  begin
    return l(0);
  end function;
end package body bl_gen_delay;

-- Generic package with a package generic and a constant: running reduction.
package bl_gen_reduce is
  generic (
    type t;
    neutral : t;
    function combine (a, b : t) return t
  );
  type arr is array (natural range <>) of t;
  function reduce (v : arr) return t;
  function scan_last (v : arr; n : natural) return t;
end package bl_gen_reduce;

package body bl_gen_reduce is
  function reduce (v : arr) return t is
    variable acc : t := neutral;
  begin
    for i in v'range loop
      acc := combine(acc, v(i));
    end loop;
    return acc;
  end function;

  function scan_last (v : arr; n : natural) return t is
    variable acc : t := neutral;
  begin
    for i in v'left to v'left + n - 1 loop
      acc := combine(acc, v(i));
    end loop;
    return acc;
  end function;
end package body bl_gen_reduce;

-- Context shared by the instances below.
context bl_inst_ctx is
  library ieee;
  use ieee.std_logic_1164.all;
  use ieee.numeric_std.all;
end context bl_inst_ctx;

-- Instances of the generic packages.
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gs_int is new work.bl_gen_stack generic map (elem => integer, capacity => 8, img => to_str);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gs_bool is new work.bl_gen_stack generic map (elem => boolean, capacity => 4, img => to_str);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gs_point is new work.bl_gen_stack generic map (elem => point_t, capacity => 6, img => to_string);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gs_op is new work.bl_gen_stack generic map (elem => opcode_t, capacity => 5, img => to_string);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gs_sl is new work.bl_gen_stack generic map (elem => std_ulogic, img => to_str);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gm_int is new work.bl_gen_minmax generic map (t => integer);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gm_real is new work.bl_gen_minmax generic map (t => real);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gm_time is new work.bl_gen_minmax generic map (t => time);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gm_char is new work.bl_gen_minmax generic map (t => character);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gm_point is new work.bl_gen_minmax generic map (t => point_t);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gm_op is new work.bl_gen_minmax generic map (t => opcode_t);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gp_int_str is new work.bl_gen_pair generic map (a_t => integer, b_t => string(1 to 4));
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gp_pt_int is new work.bl_gen_pair generic map (a_t => point_t, b_t => integer);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package ga_int_str is new work.bl_gen_assoc generic map (pairs => work.gp_int_str, same => "=");
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package ga_pt_int is new work.bl_gen_assoc generic map (pairs => work.gp_pt_int, same => "=");
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gd_int is new work.bl_gen_delay generic map (t => integer, depth => 4, init => -1);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gd_sl is new work.bl_gen_delay generic map (t => std_ulogic, depth => 3, init => '0');
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gr_sum is new work.bl_gen_reduce generic map (t => integer, neutral => 0, combine => "+");
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gr_max is new work.bl_gen_reduce generic map (t => integer, neutral => integer'low, combine => maximum);
context work.bl_inst_ctx;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_records.all;
use work.bl_enum_opcode.all;
package gr_rsum is new work.bl_gen_reduce generic map (t => real, neutral => 0.0, combine => "+");

-- Aliases of types, objects, slices, literals and subprograms.
library ieee;
use ieee.std_logic_1164.all;
use work.bl_types.all;
use work.bl_conv.all;
use work.bl_strings.all;
use work.bl_enum_opcode.all;

package bl_alias is
  alias byte_t is byte_slv;
  alias word_t is word_slv;
  alias vers is version_text;
  alias to_s is to_str [integer return string];
  alias bool_s is to_str [boolean return string];
  alias real_s is to_str [real, natural return string];
  alias trimmed is trim [string return string];
  alias plus is "+" [integer, integer return integer];
  alias idle is op_nop [return opcode_t];
  alias halted is op_halt [return opcode_t];
  constant lut : int_vec(0 to 7) := (10, 20, 30, 40, 50, 60, 70, 80);
  alias lut_low : int_vec(0 to 3) is lut(0 to 3);
  alias lut_high : int_vec(4 to 7) is lut(4 to 7);
  alias lut_all : int_vec(7 downto 0) is lut;
  function nibble_swap (v : byte_t) return byte_t;
  function high_nibble (v : byte_t) return std_logic_vector;
  function sum_halves return integer;
  function status_text (s : opcode_t) return string;
end package bl_alias;

package body bl_alias is
  function nibble_swap (v : byte_t) return byte_t is
    alias hi : std_logic_vector(3 downto 0) is v(7 downto 4);
    alias lo : std_logic_vector(3 downto 0) is v(3 downto 0);
  begin
    return lo & hi;
  end function;

  function high_nibble (v : byte_t) return std_logic_vector is
    alias hi : std_logic_vector(3 downto 0) is v(7 downto 4);
  begin
    return hi;
  end function;

  function sum_halves return integer is
    variable s : integer := 0;
  begin
    for i in lut_low'range loop
      s := plus(s, lut_low(i));
    end loop;
    for i in lut_high'range loop
      s := s - lut_high(i);
    end loop;
    return s;
  end function;

  function status_text (s : opcode_t) return string is
  begin
    if s = idle then
      return "idle";
    elsif s = halted then
      return "halted";
    end if;
    return lower(trimmed(to_string(s)));
  end function;
end package body bl_alias;

-- User-defined attributes on types, constants, functions and enumerations.
use work.bl_types.all;

package bl_attrs is
  attribute unit_label : string;
  attribute weight : integer;
  attribute doc : string;
  subtype lvl8_t is natural range 0 to 255;
  subtype lvl16_t is natural range 0 to 65535;
  type mode_t is (m_fast, m_safe, m_debug);
  constant gain_table : int_vec(0 to 3) := (1, 2, 4, 8);
  attribute unit_label of lvl8_t : subtype is "count";
  attribute unit_label of lvl16_t : subtype is "count16";
  attribute weight of gain_table : constant is 42;
  attribute weight of mode_t : type is 3;
  attribute doc of mode_t : type is "operating mode";
  function scale_gain (i : natural) return integer;
  function scale_gain (v : int_vec) return integer;
  attribute doc of scale_gain [natural return integer] : function is "picks one gain";
  function describe return string;
  function table_weight return integer;
end package bl_attrs;

package body bl_attrs is
  function scale_gain (i : natural) return integer is
  begin
    return gain_table(i mod 4);
  end function;

  function scale_gain (v : int_vec) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i) * scale_gain(i);
    end loop;
    return s;
  end function;

  function describe return string is
  begin
    return lvl8_t'unit_label & "/" & lvl16_t'unit_label & "/" & mode_t'doc;
  end function;

  function table_weight return integer is
  begin
    return gain_table'weight * mode_t'weight;
  end function;
end package body bl_attrs;

-- Check procedures with default parameters, built on the protected logger and counters.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bl_types.all;
use work.bl_prot.all;
use work.bl_conv.all;

package bl_check is
  shared variable checks : counter_bank_t;
  shared variable log : logger_t;
  procedure check (cond : boolean; msg : string := "check"; level : severity_level := error);
  procedure check_equal (got, expected : integer; msg : string := "int");
  procedure check_equal (got, expected : boolean; msg : string := "bool");
  procedure check_equal (got, expected : string; msg : string := "string");
  procedure check_equal (got, expected : std_logic_vector; msg : string := "slv");
  procedure check_equal (got, expected : unsigned; msg : string := "unsigned");
  procedure check_equal (got, expected : signed; msg : string := "signed");
  procedure check_equal (got, expected : std_ulogic; msg : string := "sl");
  procedure check_equal (got, expected : character; msg : string := "char");
  procedure check_equal (got, expected : time; msg : string := "time");
  procedure check_equal (got, expected : int_vec; msg : string := "int_vec");
  procedure check_equal (got, expected : real; tol : real := 1.0e-6; msg : string := "real");
  procedure check_between (got, lo, hi : integer; msg : string := "range");
  procedure check_not_equal (got, other : integer; msg : string := "ne");
  impure function failures return natural;
  impure function total return natural;
  procedure summary (name : string);
end package bl_check;

package body bl_check is
  procedure record_result (ok : boolean; msg : string; detail : string) is
  begin
    checks.bump("total");
    if not ok then
      checks.bump("failed");
      log.log(error, msg & ": " & detail);
      report msg & ": " & detail severity error;
    end if;
  end procedure;

  procedure check (cond : boolean; msg : string := "check"; level : severity_level := error) is
  begin
    checks.bump("total");
    if not cond then
      checks.bump("failed");
      log.log(level, msg);
      report "check failed: " & msg severity level;
    end if;
  end procedure;

  procedure check_equal (got, expected : integer; msg : string := "int") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : boolean; msg : string := "bool") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : string; msg : string := "string") is
  begin
    record_result(got = expected, msg, got & " /= " & expected);
  end procedure;

  procedure check_equal (got, expected : std_logic_vector; msg : string := "slv") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : unsigned; msg : string := "unsigned") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : signed; msg : string := "signed") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : std_ulogic; msg : string := "sl") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : character; msg : string := "char") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : time; msg : string := "time") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : int_vec; msg : string := "int_vec") is
  begin
    record_result(got = expected, msg, to_str(got) & " /= " & to_str(expected));
  end procedure;

  procedure check_equal (got, expected : real; tol : real := 1.0e-6; msg : string := "real") is
  begin
    record_result(abs (got - expected) <= tol, msg, to_str(got, 6) & " /= " & to_str(expected, 6));
  end procedure;

  procedure check_between (got, lo, hi : integer; msg : string := "range") is
  begin
    record_result(got >= lo and got <= hi, msg, to_str(got) & " outside " & to_str(lo) & ".." & to_str(hi));
  end procedure;

  procedure check_not_equal (got, other : integer; msg : string := "ne") is
  begin
    record_result(got /= other, msg, to_str(got) & " = " & to_str(other));
  end procedure;

  impure function failures return natural is
  begin
    return checks.value("failed");
  end function;

  impure function total return natural is
  begin
    return checks.value("total");
  end function;

  procedure summary (name : string) is
  begin
    report name & ": " & to_str(total) & " checks, " & to_str(failures) & " failed" severity note;
    assert failures = 0 report name & " has failures" severity failure;
  end procedure;
end package body bl_check;

-- Procedures with signal, variable, constant and file class parameters.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
use work.bl_types.all;

package bl_sigproc is
  procedure clock_gen (signal clk : out std_logic; period : time; cycles : natural);
  procedure wait_cycles (signal clk : in std_logic; n : natural);
  procedure pulse (signal s : out std_logic; width : time; active : std_logic := '1');
  procedure wait_high (signal s : in std_logic; timeout : time; ok : out boolean);
  procedure sample_n (signal clk : in std_logic; signal d : in std_logic_vector; variable v : inout int_vec; n : natural);
  procedure drive_seq (signal clk : in std_logic; signal d : out std_logic_vector; constant seq : in int_vec);
  procedure count_edges (signal s : in std_logic; window : time; variable n : out natural);
  procedure bump (variable v : inout natural; by : natural := 1);
  procedure dump_seq (file f : text; constant seq : in int_vec);
end package bl_sigproc;

package body bl_sigproc is
  procedure clock_gen (signal clk : out std_logic; period : time; cycles : natural) is
  begin
    for i in 1 to cycles loop
      clk <= '1';
      wait for period / 2;
      clk <= '0';
      wait for period / 2;
    end loop;
  end procedure;

  procedure wait_cycles (signal clk : in std_logic; n : natural) is
  begin
    for i in 1 to n loop
      wait until rising_edge(clk);
    end loop;
  end procedure;

  procedure pulse (signal s : out std_logic; width : time; active : std_logic := '1') is
  begin
    s <= active;
    wait for width;
    s <= not active;
  end procedure;

  procedure wait_high (signal s : in std_logic; timeout : time; ok : out boolean) is
  begin
    if s = '1' then
      ok := true;
      return;
    end if;
    wait until s = '1' for timeout;
    ok := s = '1';
  end procedure;

  procedure sample_n (signal clk : in std_logic; signal d : in std_logic_vector; variable v : inout int_vec; n : natural) is
  begin
    for i in 0 to n - 1 loop
      wait until rising_edge(clk);
      v(v'left + i) := to_integer(unsigned(d));
    end loop;
  end procedure;

  procedure drive_seq (signal clk : in std_logic; signal d : out std_logic_vector; constant seq : in int_vec) is
  begin
    for i in seq'range loop
      d <= std_logic_vector(to_unsigned(seq(i), d'length));
      wait until rising_edge(clk);
    end loop;
  end procedure;

  procedure count_edges (signal s : in std_logic; window : time; variable n : out natural) is
    variable c : natural := 0;
    constant t_end : time := now + window;
  begin
    while now < t_end loop
      wait on s for t_end - now;
      if s'event then
        c := c + 1;
      end if;
    end loop;
    n := c;
  end procedure;

  procedure bump (variable v : inout natural; by : natural := 1) is
  begin
    v := v + by;
  end procedure;

  procedure dump_seq (file f : text; constant seq : in int_vec) is
    variable l : line;
  begin
    for i in seq'range loop
      write(l, seq(i));
      write(l, ' ');
    end loop;
    writeline(f, l);
  end procedure;
end package body bl_sigproc;

-- Entities with generate statements and blocks.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity bl_stage is
  generic (width : positive := 8);
  port (
    clk : in std_logic;
    d : in std_logic_vector(width - 1 downto 0);
    q : out std_logic_vector(width - 1 downto 0)
  );
end entity bl_stage;

architecture rtl of bl_stage is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;

entity bl_pipe is
  generic (width : positive := 8; stages : positive := 3);
  port (
    clk : in std_logic;
    d : in std_logic_vector(width - 1 downto 0);
    q : out std_logic_vector(width - 1 downto 0)
  );
end entity bl_pipe;

architecture gen of bl_pipe is
  type link_arr is array (0 to stages) of std_logic_vector(width - 1 downto 0);
  signal links : link_arr;
begin
  links(0) <= d;
  g_stage : for i in 0 to stages - 1 generate
    u_stage : entity work.bl_stage
      generic map (width => width)
      port map (clk => clk, d => links(i), q => links(i + 1));
  end generate g_stage;
  q <= links(stages);
end architecture gen;

library ieee;
use ieee.numeric_std.all;

entity bl_sel is
  generic (kind : natural range 0 to 3 := 0; width : positive := 8);
  port (
    a, b : in unsigned(width - 1 downto 0);
    y : out unsigned(width - 1 downto 0)
  );
end entity bl_sel;

architecture sel of bl_sel is
begin
  g_kind : case kind generate
    when 0 =>
      y <= a + b;
    when 1 =>
      y <= a - b;
    when 2 =>
      y <= a and b;
    when others =>
      y <= a or b;
  end generate g_kind;
end architecture sel;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity bl_scale is
  generic (shift_by : natural := 0; saturate : boolean := false);
  port (x : in unsigned(7 downto 0); y : out unsigned(7 downto 0));
end entity bl_scale;

architecture a of bl_scale is
begin
  g_pass : if shift_by = 0 generate
    y <= x;
  elsif saturate generate
    b_sat : block
      signal wide : unsigned(15 downto 0) := (others => '0');
    begin
      wide <= shift_left(resize(x, 16), shift_by);
      y <= x"FF" when wide > 255 else wide(7 downto 0);
    end block b_sat;
  else generate
    y <= shift_left(x, shift_by);
  end generate g_pass;
end architecture a;

library ieee;
use ieee.std_logic_1164.all;

entity bl_gate is
  port (en : in std_logic; d : in std_logic_vector(3 downto 0); q : out std_logic_vector(3 downto 0));
end entity bl_gate;

architecture gated of bl_gate is
begin
  blk : block (en = '1')
  begin
    q <= guarded d;
  end block blk;
end architecture gated;

-- Testbench: calls a sample of the library and prints PASS.
library ieee;
context work.bl_inst_ctx;
use std.textio.all;
use work.bl_types.all;
use work.bl_chars.all;
use work.bl_strings.all;
use work.bl_conv.all;
use work.bl_vec.all;
use work.bl_records.all;
use work.bl_timeutil.all;
use work.bl_math.all;
use work.bl_bits.all;
use work.bl_crc.all;
use work.bl_calendar.all;
use work.bl_text.all;
use work.bl_prot.all;
use work.bl_check.all;
use work.bl_sigproc.all;
use work.bl_alias.all;
use work.bl_attrs.all;
use work.bl_enum_opcode.all;
use work.bl_enum_weekday.all;
use work.bl_enum_month.all;
use work.bl_linked.node_ptr;
use work.bl_linked.tree_ptr;
use work.bl_linked.str_node_ptr;
use work.gs_int;
use work.gs_op;
use work.gm_int;
use work.gm_real;
use work.gm_time;
use work.gm_char;
use work.gm_point;
use work.gp_int_str;
use work.ga_int_str;
use work.gd_int;
use work.gr_sum;
use work.gr_max;

entity big_lib_tb is
end entity big_lib_tb;

architecture tb of big_lib_tb is
  signal clk : std_logic := '0';
  signal d, q : std_logic_vector(7 downto 0) := (others => '0');
  signal strobe : std_logic := '0';
  signal sa, sb : unsigned(7 downto 0) := to_unsigned(12, 8);
  type y_arr is array (0 to 3) of unsigned(7 downto 0);
  signal y : y_arr;
  signal sc1, sc2, sc3 : unsigned(7 downto 0);
  signal gate_en : std_logic := '0';
  signal gate_q : std_logic_vector(3 downto 0);
  signal mon_done : boolean := false;
  shared variable mon_q : int_queue_t;
  shared variable rng : rng_t;
  shared variable stk : int_stack_t;
  shared variable imap : int_map_t;
  shared variable pool : string_pool_t;
  shared variable st : stats_t;
  constant ten : integer := 10;

  function str_bytes (s : string) return byte_array is
    variable r : byte_array(0 to s'length - 1);
  begin
    for i in r'range loop
      r(i) := std_logic_vector(to_unsigned(character'pos(s(s'left + i)), 8));
    end loop;
    return r;
  end function;

  procedure t_chars is
  begin
    check(is_digit('7') and not is_digit('a') and is_alpha('q') and is_alnum('5'), "char classes");
    check_equal(to_upper('q'), 'Q', "upper");
    check_equal(to_lower('Q'), 'q', "lower");
    check_equal(digit_value('9'), 9, "digit");
    check_equal(hex_value('f'), 15, "hex f");
    check_equal(hex_value('G'), -1, "hex G");
    check_equal(hex_char(11, true), 'B', "hex_char upper");
    check_equal(hex_char(11), 'b', "hex_char");
    check(classify('x') = cc_lower and classify('X') = cc_upper and classify('5') = cc_digit, "classify 1");
    check(classify(' ') = cc_space and classify('!') = cc_punct and classify(NUL) = cc_control, "classify 2");
    check_equal(to_sl('z'), 'Z', "to_sl");
    check_equal(to_char('H'), 'H', "to_char");
    check(is_vowel('E') and not is_vowel('Z') and is_whitespace(HT), "vowel");
    check_equal(rot('a', 13), 'n', "rot a");
    check_equal(rot('Z', 1), 'A', "rot Z");
    check_equal(rot('m', -13), 'z', "rot m");
    check_equal(next_char('a'), 'b', "next_char");
    check_equal(char_code('A'), 65, "char_code");
    check_equal(code_char(66), 'B', "code_char");
  end procedure;

  procedure t_strings is
    constant sv : string_vec := ("ab", "cd", "ef");
  begin
    check_equal(upper("abC1"), "ABC1", "upper");
    check_equal(lower("AbC"), "abc", "lower");
    check_equal(trim("  hi  "), "hi", "trim");
    check_equal(trim_left("  hi "), "hi ", "trim_left");
    check_equal(trim_right(" hi  "), " hi", "trim_right");
    check_equal(trim("xxabxx", 'x'), "ab", "trim char");
    check(starts_with("hello", "he") and ends_with("hello", "llo") and not starts_with("he", "hello"), "affix");
    check(contains("hello", 'l') and contains("hello", "ell") and not contains("hello", 'z'), "contains");
    check_equal(index_of("hello", 'l'), 3, "index_of");
    check_equal(index_of("hello", 'l', 4), 4, "index_of from");
    check_equal(index_of("hello", "lo"), 4, "index_of str");
    check_equal(index_of("hello", "z"), 0, "index_of none");
    check_equal(count("banana", 'a'), 3, "count char");
    check_equal(count("banana", "an"), 2, "count str");
    check_equal(replace("a-b-c", '-', '+'), "a+b+c", "replace char");
    check_equal(replace("one two one", "one", "1"), "1 two 1", "replace str");
    check_equal(reverse("abc"), "cba", "reverse");
    check_equal(repeat("ab", 3), "ababab", "repeat str");
    check_equal(repeat('x', 4), "xxxx", "repeat char");
    check_equal(pad_left("7", 3, '0'), "007", "pad_left");
    check_equal(pad_right("ab", 4), "ab  ", "pad_right");
    check_equal(center("ab", 6), "  ab  ", "center");
    check_equal(substr("abcdef", 2, 3), "bcd", "substr");
    check_equal(head("abcdef", 2) & tail("abcdef", 2), "abef", "head tail");
    check_equal(field_count("a,b,,c", ','), 4, "field_count");
    check_equal(field("a,b,,c", ',', 2) & field("a,b,,c", ',', 4), "bc", "field");
    check_equal(join(sv, "-"), "ab-cd-ef", "join str");
    check_equal(join(sv, '+'), "ab+cd+ef", "join char");
    check(is_number("-123") and not is_number("12a") and is_blank("  ") and equal_nocase("AbC", "aBc"), "predicates");
    check_equal(compare("abc", "abd") + compare("b", "a") + compare("a", "ab") + compare("a", "a"), -1, "compare");
    check_equal(capitalize("hello woRLD"), "Hello World", "capitalize");
    check_equal(squeeze("a   b  c"), "a b c", "squeeze");
    check_equal(rot13("Hello"), "Uryyb", "rot13");
    check_equal(hash("abc"), 485384, "hash");
    check_equal(vowels("education"), 5, "vowels");
    check_equal(longest(string_vec'("ab ", "abc", "x  ")), 3, "longest");
    check_equal(to_slv_str("10X1"), std_logic_vector'("10X1"), "to_slv_str");
    check_equal(from_slv_str(std_logic_vector'("01Z")), "01Z", "from_slv_str");
    check_equal(words("  the quick  fox "), 3, "words");
    check_equal(abbreviate("abcdefghij", 8), "abcde...", "abbreviate");
    check_equal(unquote(quote("q")), "q", "quote");
    check_equal(plural(1, "cat", "cats") & "," & plural(3, "cat", "cats"), "1 cat,3 cats", "plural");
  end procedure;

  procedure t_conv is
    constant w : std_logic_vector(31 downto 0) := x"11223344";
    constant bytes : byte_array := bytes_of(w);
  begin
    check_equal(to_str(42) & to_str(-5), "42-5", "to_str int");
    check_equal(to_str(true) & to_str(false), "truefalse", "to_str bool");
    check_equal(to_str(3.14159, 2), "+3.14", "to_str real");
    check_equal(to_str(-2.5, 1), "-2.5", "to_str neg real");
    check_equal(to_str(std_ulogic'('1')) & to_str(character'('a')), "1a", "to_str sl char");
    check_equal(to_str(std_logic_vector'("10Z")) & to_str(unsigned'("01")) & to_str(signed'("1")), "10Z011", "to_str vec");
    check_equal(to_str(bit_vector'("101")), "101", "to_str bv");
    check_equal(to_str(int_vec'(1, -2, 3)), "[1,-2,3]", "to_str int_vec");
    check_equal(to_str(bool_vec'(true, false)), "<TF>", "to_str bool_vec");
    check_equal(to_hex(std_logic_vector'(x"BEEF")), "BEEF", "to_hex slv");
    check_equal(to_hex(unsigned'(x"0A")), "0A", "to_hex uns");
    check_equal(to_hex(255, 4), "00FF", "to_hex nat");
    check_equal(to_bin(5, 4), "0101", "to_bin");
    check_equal(to_int(string'("-42")), -42, "to_int str");
    check_equal(to_int(true) + to_int(bit'('1')), 2, "to_int bool bit");
    check_equal(to_int(std_logic_vector'("1010")) + to_int(bit_vector'("0101")), 15, "to_int vec");
    check(to_bool("true") and not to_bool(0) and to_bool('H') and to_bool(5), "to_bool");
    check_equal(to_sl(true), '1', "to_sl bool");
    check_equal(to_slv(5, 4), std_logic_vector'("0101"), "to_slv int");
    check_equal(to_slv(-1, 4), std_logic_vector'("1111"), "to_slv neg");
    check_equal(to_slv(bool_vec'(true, false, true)), std_logic_vector'("101"), "to_slv bools");
    check_equal(to_slv(bit_vector'("1001")), std_logic_vector'("1001"), "to_slv bv");
    check_equal(to_uns(5, 4), to_unsigned(5, 4), "to_uns");
    check_equal(to_integer(to_sgn(-3, 4)), -3, "to_sgn");
    check_equal(to_int(to_bitvec(5, 4)), 5, "to_bitvec");
    check_equal(to_real(7), 7.0, 1.0e-9, "to_real int");
    check_equal(to_real("2.5"), 2.5, 1.0e-9, "to_real str");
    check_equal(to_real(1500 ps), 1.5, 1.0e-9, "to_real time");
    check_equal(to_time_ns(2.5), 2500 ps, "to_time_ns real");
    check_equal(to_time_ns(3), 3 ns, "to_time_ns int");
    check_equal(to_ints(std_logic_vector'("101"))(0), 1, "to_ints");
    check_equal(sum(to_reals(int_vec'(1, 2, 3))), 6.0, 1.0e-9, "to_reals");
    check_equal(round_to_int(2.5) + round_to_int(-2.5), 0, "round");
    check_equal(parse_hex("A5"), std_logic_vector'("10100101"), "parse_hex");
    check_equal(to_integer(sat_resize(to_signed(100, 8), 4)), 7, "sat hi");
    check_equal(to_integer(sat_resize(to_signed(-100, 8), 4)), -8, "sat lo");
    check_equal(to_integer(sat_resize(to_signed(3, 8), 4)), 3, "sat mid");
    check_equal(bytes(0), x"44", "bytes 0");
    check_equal(bytes(3), x"11", "bytes 3");
    check_equal(word_of(bytes), w, "word_of");
    check_equal(swap_bytes(w), x"44332211", "swap_bytes");
  end procedure;

  procedure t_vec is
    constant v : int_vec := (5, 3, 9, 1, 7);
    constant r : real_vec := (1.0, 2.0, 6.0);
  begin
    check_equal(sum(v), 25, "sum");
    check_equal(product(int_vec'(1, 2, 3, 4)), 24, "product");
    check_equal(max_of(v) - min_of(v), 8, "max min");
    check_equal(mean(r), 3.0, 1.0e-9, "mean");
    check_equal(variance(r), 14.0 / 3.0, 1.0e-9, "variance");
    check_equal(max_of(r) + min_of(r), 7.0, 1.0e-9, "real max min");
    check_equal(index_of(v, 9), 2, "index_of");
    check(contains(v, 7) and not contains(v, 8), "contains");
    check_equal(count_true(bool_vec'(true, false, true)), 2, "count_true");
    check(any_true(bool_vec'(false, true)) and not all_true(bool_vec'(false, true)), "any all");
    check_equal(reverse(v), int_vec'(7, 1, 9, 3, 5), "reverse");
    check_equal(sorted(v), int_vec'(1, 3, 5, 7, 9), "sorted");
    check(is_sorted(sorted(v)) and not is_sorted(v), "is_sorted");
    check_equal(iota(4, 10), int_vec'(10, 11, 12, 13), "iota");
    check_equal(span(2, 10, 3), int_vec'(2, 5, 8), "span");
    check_equal(cumsum(int_vec'(1, 2, 3, 4)), int_vec'(1, 3, 6, 10), "cumsum");
    check_equal(diffs(v), int_vec'(-2, 6, -8, 6), "diffs");
    check_equal(positives(int_vec'(-1, 2, 0, 3)), int_vec'(2, 3), "positives");
    check_equal(unique(int_vec'(1, 2, 1, 3, 2)), int_vec'(1, 2, 3), "unique");
    check_equal(v * 2, int_vec'(10, 6, 18, 2, 14), "scale");
    check_equal(dot(int_vec'(1, 2, 3), int_vec'(4, 5, 6)), 32, "dot");
    check_equal(median(v), 5, "median");
    check_equal(v + v - v, v, "plus minus");
    check_equal(-v + 1, int_vec'(-4, -2, -8, 0, -6), "neg add");
    check(int_vec'(1, 2) < int_vec'(1, 3) and v > sorted(v) and int_vec'(1 => 1) <= int_vec'(1 => 1), "vec compare");
    check((bool_vec'(true, true) and bool_vec'(true, false)) = bool_vec'(true, false), "bool and");
    check(((not bool_vec'(true, false)) or bool_vec'(false, false)) = bool_vec'(false, true), "bool not or");
  end procedure;

  procedure t_records is
    constant p1 : point_t := (1, 2);
    constant p2 : point_t := (3, 4);
    constant pp : point_vec := p1 & p2;
    constant i1 : interval_t := (1, 5);
    constant i2 : interval_t := (4, 9);
    constant ca : complex_t := (1.0, 2.0);
    constant cb : complex_t := (3.0, -1.0);
  begin
    check(p1 + p2 = (4, 6) and p2 - p1 = (2, 2) and p1 * 3 = (3, 6) and -p1 = (-1, -2), "point ops");
    check(p1 < p2 and not (p2 < p1) and pp(1) = p2, "point order concat");
    check_equal(manhattan(p1, p2), 4, "manhattan");
    check_equal(to_string(p1), "(1,2)", "point string");
    check(bounding_box((p1, p2, origin)) = (0, 3), "bounding_box");
    check(centroid((p1, p2, (5, 6))) = (3, 4), "centroid");
    check(half + half = to_fraction(1) and half * to_fraction(2, 3) = to_fraction(1, 3), "fraction add mul");
    check(half < to_fraction(2, 3) and half >= to_fraction(2, 4) and half /= to_fraction(1, 3), "fraction compare");
    check_equal(to_string(to_fraction(6, 8)), "3/4", "fraction string");
    check_equal(to_real(half), 0.5, 1.0e-9, "fraction real");
    check(abs to_fraction(-3, 4) = to_fraction(3, 4) and -half = to_fraction(-1, 2), "fraction abs neg");
    check_equal(to_string(half / to_fraction(1, 4)), "2/1", "fraction div");
    check(to_fraction(1, 2) - to_fraction(1, 3) = to_fraction(1, 6), "fraction sub");
    check(ca * cb = (5.0, 5.0) and ca + cb = (4.0, 1.0) and ca - cb = (-2.0, 3.0), "complex ops");
    check_equal(abs (3.0, 4.0), 5.0, 1.0e-9, "complex abs");
    check(conj(ca) = (1.0, -2.0) and imag_unit * imag_unit = (-1.0, 0.0), "complex conj");
    check(cpow((1.0, 1.0), 4) = (-4.0, 0.0) and ca * 2.0 = (2.0, 4.0), "complex pow");
    check_equal(arg(imag_unit), 1.5707963, 1.0e-6, "complex arg");
    check(polar(2.0, 0.0) = (2.0, 0.0), "polar");
    check_equal(interval_width(i1), 5, "width");
    check(overlaps(i1, i2) and contains(i1, 3) and not contains(i1, 6) and is_empty((6, 5)), "interval preds");
    check((i1 and i2) = (4, 5) and (i1 or i2) = (1, 9) and i1 + i2 = (5, 14), "interval ops");
    check_equal(brightness(white), 255, "brightness");
    check(mix(black, white) = (127, 127, 127) and unpack(pack((1, 2, 3))) = (1, 2, 3), "rgb");
  end procedure;

  procedure t_time_math is
  begin
    check_equal(tmin(3 ns, 5 ns), 3 ns, "tmin");
    check_equal(tmax(3 ns, 5 ns), 5 ns, "tmax");
    check_equal(limit(10 ns, 1 ns, 5 ns), 5 ns, "limit");
    check_equal(limit_pos(-3 ns), 0 ns, "limit_pos");
    check_equal(to_ns(12 ns) + to_ps(1 ns), 1012, "to_ns to_ps");
    check_equal(from_ns(7), 7 ns, "from_ns");
    check_equal(from_hz(1000), 1 ms, "from_hz");
    check_equal(to_hz(10 ns), 100000000, "to_hz");
    check_equal(round_to(17 ns, 5 ns), 15 ns, "round_to");
    check(is_multiple(20 ns, 5 ns) and not is_multiple(21 ns, 5 ns), "is_multiple");
    check_equal(half_period(10 ns) + scale_time(3 ns, 4), 17 ns, "half scale");
    check_equal(cycles_in(1 us, 10 ns), 100, "cycles_in");
    check_equal(earliest((4 ns, 1 ns, 3 ns)) + latest((4 ns, 1 ns, 3 ns)), 5 ns, "earliest latest");
    check_equal(total((4 ns, 1 ns, 3 ns)), 8 ns, "total");
    check_equal(unit_name(5 us), "us", "unit_name");
    check_equal(to_us_real(2500 ns), 2.5, 1.0e-9, "to_us_real");
    check_equal(gcd(48, 18) + lcm(4, 6), 18, "gcd lcm");
    check_equal(fib(15), 610, "fib");
    check_equal(factorial(6), 720, "factorial");
    check_equal(ipow(3, 5) + ipow(-2, 3), 235, "ipow");
    check_equal(isqrt(99), 9, "isqrt");
    check_equal(ilog2(1000) + clog2(1000) + clog2(1024), 29, "logs");
    check_equal(binomial(10, 3), 120, "binomial");
    check(is_prime(97) and not is_prime(91) and next_prime(100) = 101, "primes");
    check_equal(collatz_steps(27), 111, "collatz");
    check_equal(digit_sum(98765) + digit_count(12345), 40, "digits");
    check_equal(ackermann(2, 3), 9, "ackermann");
    check_equal(clamp(15, 0, 10), 10, "clamp int");
    check_equal(clamp(-1.5, 0.0, 1.0), 0.0, 1.0e-9, "clamp real");
    check_equal(sign_of(-9) + sign_of(2.5), 0, "sign_of");
    check_equal(mod_pow(3, 200, 13), 9, "mod_pow");
    check(approx(0.1 + 0.2, 0.3) and not approx(1.0, 1.1), "approx");
    check_equal(lerp(0.0, 10.0, 0.25), 2.5, 1.0e-9, "lerp");
    check_equal(to_db(10.0), 20.0, 1.0e-9, "to_db");
    check_equal(triangle(10), 55, "triangle");
    check(is_power_of_two(64) and not is_power_of_two(48), "pow2");
    check_equal(round_up(13, 8), 16, "round_up");
    check_equal(poly_eval((1.0, 0.0, -1.0), 3.0), 8.0, 1.0e-9, "poly_eval");
    check_equal(newton_sqrt(2.0), 1.41421356, 1.0e-6, "newton_sqrt");
  end procedure;

  procedure t_bits is
    constant x8 : std_logic_vector(7 downto 0) := x"A5";
  begin
    check_equal(popcount(std_logic_vector'(x"F0F0")), 8, "popcount slv");
    check_equal(popcount(unsigned'("0111")) + popcount(bit_vector'("11")), 5, "popcount uns bv");
    check_equal(parity(std_logic_vector'("1011")), '1', "parity");
    check_equal(reverse_bits(std_logic_vector'("0011")), std_logic_vector'("1100"), "reverse_bits");
    check_equal(reverse_bits(unsigned'("0011")), unsigned'("1100"), "reverse_bits uns");
    check_equal(rotl(std_logic_vector'("1001"), 1), std_logic_vector'("0011"), "rotl");
    check_equal(rotr(std_logic_vector'("1001"), 1), std_logic_vector'("1100"), "rotr");
    check_equal(to_gray(unsigned'("0101")), unsigned'("0111"), "to_gray");
    check_equal(from_gray(unsigned'("0111")), unsigned'("0101"), "from_gray");
    check(is_onehot(std_logic_vector'("00100")) and not is_onehot(std_logic_vector'("00101")), "onehot");
    check_equal(onehot_index(std_logic_vector'("00100")), 2, "onehot_index");
    check_equal(to_onehot(3, 8), std_logic_vector'("00001000"), "to_onehot");
    check_equal(clz(unsigned'("00010000")) + ctz(unsigned'("00010000")) + msb_index(unsigned'("00010000")), 11, "clz ctz msb");
    check_equal(mask_low(3, 8), std_logic_vector'("00000111"), "mask_low");
    check_equal(extract(x"ABCD", 4, 8), x"BC", "extract");
    check_equal(insert(x"0000", std_logic_vector'("1111"), 4), x"00F0", "insert");
    check_equal(hamming(std_logic_vector'("1100"), std_logic_vector'("1010")), 2, "hamming");
    check_equal(interleave(std_logic_vector'("11"), std_logic_vector'("00")), std_logic_vector'("1010"), "interleave");
    check_equal(sign_extend(std_logic_vector'("1000"), 8), std_logic_vector'("11111000"), "sign_extend");
    check_equal(zero_extend(std_logic_vector'("1000"), 8), std_logic_vector'("00001000"), "zero_extend");
    check_equal(bit_at(std_logic_vector'("0100"), 2), '1', "bit_at");
    check_equal(set_bit(std_logic_vector'("0000"), 3), std_logic_vector'("1000"), "set_bit");
    check_equal(shift_in(std_logic_vector'("1001"), '1'), std_logic_vector'("0011"), "shift_in");
    check(any_set(x8) and not any_set(std_logic_vector'("0000")), "any_set");
    check_equal(crc8(str_bytes("123456789")), x"F4", "crc8");
    check_equal(crc8("123456789"), x"F4", "crc8 string");
    check_equal(crc16(str_bytes("123456789")), x"29B1", "crc16");
    check_equal(adler32("Wikipedia"), 300286872, "adler32");
    check_equal(fcheck16(str_bytes("abcde")), 51440, "fcheck16");
  end procedure;

  procedure t_calendar_enums is
    constant ops : opcode_vec := all_values;
    constant days : weekday_vec := all_values;
    constant s : opcode_set := make_set((op_nop, op_halt, op_nop));
  begin
    check(is_leap(2000) and not is_leap(1900) and is_leap(2024), "leap");
    check_equal(days_in(2023, feb) + days_in(2024, feb) + days_in(2024, apr), 87, "days_in");
    check_equal(day_of_year((2024, mar, 1)), 61, "day_of_year");
    check(weekday_of((1970, jan, 1)) = thursday and weekday_of((2000, jan, 1)) = saturday, "weekday 1");
    check(weekday_of((2024, mar, 1)) = friday and is_weekend(sunday), "weekday 2");
    check(next_day((2023, dec, 31)) = (2024, jan, 1), "next_day");
    check(add_days((2024, feb, 27), 3) = (2024, mar, 1), "add_days");
    check_equal(days_between((2024, jan, 1), (2024, dec, 31)), 365, "days_between");
    check(valid((2024, feb, 29)) and not valid((2023, feb, 29)), "valid");
    check(equal_nocase(to_string(date_t'(2024, mar, 1)), "2024-mar-1"), "date string");
    check(next_opcode(op_halt) = op_nop and prev_opcode(op_nop) = op_halt, "next prev");
    check(equal_nocase(to_string(op_add), "op_add") and parse_opcode("op_jmp") = op_jmp, "image value");
    check_equal(to_int(op_jz) + to_int(to_opcode(17)), 12, "pos val");
    check_equal(distance(op_nop, op_halt), 15, "distance");
    check(between(op_add, op_load, op_sub) and not between(op_or, op_load, op_sub), "between");
    check(op_nop + 5 = op_and and op_halt + 1 = op_nop and op_add - 2 = op_load and op_halt - op_nop = 15, "enum ops");
    check_equal(ops'length, 16, "all_values opcode");
    check_equal(days'length, 7, "all_values weekday");
    check_equal(members(s), 2, "set members");
    check(member(op_halt, s) and not member(op_add, s), "set member");
    check_equal(weight_of(op_xor), opcode_weights(op_xor), "weight_of");
    check(is_first(op_nop) and is_last(op_halt) and not is_first(op_add), "first last");
    check(starts_with(lower(all_opcode_names), "op_nop,op_load"), "all names");
    check(next_opcode(op_halt) = op_nop and prev_month(jan) = dec and next_weekday(sunday) = monday, "wrap");
    check(opcode_low_t'right = op_xor and month_low_t'right = jun, "subtype bounds");
    check(status_text(op_nop) = "idle" and status_text(op_halt) = "halted" and status_text(op_xor) = "op_xor", "status_text");
  end procedure;

  procedure t_text is
    variable l : line;
    variable w : string(1 to 6);
    variable n : natural;
    variable v : integer;
    variable ok : boolean;
    variable h : std_logic_vector(15 downto 0);
  begin
    l := line_of("alpha 12 ff x");
    next_word(l, w, n);
    check_equal(w(1 to n), "alpha", "next_word");
    next_int(l, v, ok);
    check(ok and v = 12, "next_int");
    next_hex(l, h, ok);
    check(ok and h = x"00FF", "next_hex");
    next_int(l, v, ok);
    check(not ok, "next_int bad");
    drop(l);
    check_equal(sum_of_ints("1 2 3 4 -5"), 5, "sum_of_ints");
    check_equal(words_in("a bb  ccc"), 3, "words_in");
    put(l, string'("x="));
    put(l, 5);
    put(l, string'(" ok="));
    put(l, true);
    put(l, string'(" "));
    put_hex(l, x"0F");
    put(l, std_logic_vector'("101"));
    check_equal(l.all, "x=5 ok=true 0x0F101", "put");
    emit(output, l);
    emit_kv(output, "answer", 42);
    emit_table(output, iota(10), 5);
    emit(output, string'("done"));
  end procedure;

  procedure t_linked is
    variable head : node_ptr;
    variable root : tree_ptr;
    variable texts : str_node_ptr;
    variable s : str_ptr;
    variable n : natural;
    variable total : integer;
    variable found : boolean;
    variable flat : int_vec(0 to 6);
    variable cnt : natural := 0;
    variable vec : int_vec(0 to 3);
  begin
    for i in 1 to 3 loop
      work.bl_linked.push_front(head, i * 10);
    end loop;
    work.bl_linked.append(head, 5);
    work.bl_linked.length(head, n);
    work.bl_linked.sum(head, total);
    check(n = 4 and total = 65, "list length sum");
    work.bl_linked.nth(head, 1, total);
    work.bl_linked.find(head, 5, found);
    check(total = 20 and found, "list nth find");
    work.bl_linked.to_vec(head, vec);
    check_equal(vec, int_vec'(30, 20, 10, 5), "list to_vec");
    work.bl_linked.reverse_list(head);
    work.bl_linked.remove_value(head, 20);
    work.bl_linked.to_vec(head, vec(0 to 2));
    check_equal(vec(0 to 2), int_vec'(5, 10, 30), "list reverse remove");
    work.bl_linked.free_list(head);
    check(head = null, "list freed");
    for i in 0 to 6 loop
      work.bl_linked.tree_insert(root, (i * 5) mod 7 + 1);
    end loop;
    work.bl_linked.tree_size(root, n);
    work.bl_linked.tree_height(root, total);
    check(n = 7 and total >= 3, "tree size height");
    work.bl_linked.tree_flatten(root, flat, cnt);
    check_equal(flat, int_vec'(1, 2, 3, 4, 5, 6, 7), "tree flatten");
    work.bl_linked.tree_min(root, n);
    work.bl_linked.tree_max(root, total);
    work.bl_linked.tree_contains(root, 4, found);
    check(n = 1 and total = 7 and found, "tree min max contains");
    work.bl_linked.tree_free(root);
    work.bl_linked.add_text(texts, "one");
    work.bl_linked.add_text(texts, "two");
    work.bl_linked.text_count(texts, n);
    work.bl_linked.text_at(texts, 1, s);
    check(n = 2 and s.all = "two", "string list");
    work.bl_linked.free_texts(texts);
  end procedure;

  procedure t_prot is
    variable acc : natural := 0;
  begin
    for i in 1 to 5 loop
      mon_q.push(i * i);
    end loop;
    check_equal(mon_q.pop + mon_q.pop, 5, "queue order");
    check_equal(mon_q.size, 3, "queue size");
    check(not mon_q.is_empty and not mon_q.is_full and mon_q.peek = 9, "queue peek");
    mon_q.clear;
    check(mon_q.is_empty, "queue clear");
    check_equal(mon_q.total_pushed, 5, "queue pushed");
    stk.push(4);
    stk.push(8);
    check_equal(stk.top + stk.depth, 10, "stack");
    check_equal(stk.snapshot, int_vec'(4, 8), "stack snapshot");
    check_equal(stk.pop - stk.pop, 4, "stack pop");
    for i in 0 to 20 loop
      imap.put(i * 3, i);
    end loop;
    imap.remove(9);
    check(imap.has(30) and not imap.has(9) and imap.size = 20, "map");
    check_equal(imap.get(30) + imap.get(31, 100), 110, "map get");
    check_equal(pool.intern("red") + pool.intern("green") + pool.intern("red"), 1, "pool intern");
    check_equal(pool.get(1) & integer'image(pool.count), "green2", "pool get");
    rng.seed(42);
    for i in 1 to 200 loop
      acc := acc + rng.uniform(1, 6);
      st.add(real(rng.uniform(0, 100)));
    end loop;
    check_between(acc, 600, 800, "rng average");
    check_between(rng.bits(8), 0, 255, "rng bits");
    check(rng.real01 < 1.0, "rng real01");
    check_between(integer(st.mean), 35, 65, "stats mean");
    check(st.n = 200 and st.min_value >= 0.0 and st.max_value <= 100.0 and st.stddev > 10.0, "stats");
    log.log(warning, "careful");
    log.log(error, "broken");
    check_equal(log.count(warning) + log.count(error), 2, "logger count");
    check_equal(log.last_message, "broken", "logger last");
    check_equal(checks.value("total") - checks.value("failed"), total - failures, "counter bank");
    check_equal(checks.names, 1, "counter names");
  end procedure;

  procedure t_generics is
    variable si : gs_int.stack_t := (items => (others => 0), sp => 0);
    variable so : gs_op.stack_t := (items => (others => op_nop), sp => 0);
    variable e : integer;
    variable ia : gm_int.arr(0 to 4) := (5, 3, 9, 1, 7);
    variable ra : gm_real.arr(0 to 2) := (2.5, -1.0, 4.0);
    variable ca : gm_char.arr(0 to 2) := ('q', 'a', 'z');
    variable pa : gm_point.arr(0 to 2) := ((3, 4), (1, 9), (1, 2));
    constant tab : gp_int_str.pair_arr := (gp_int_str.make(1, "one "), gp_int_str.make(2, "two "));
    variable dl : gd_int.line_t := gd_int.empty_line;
  begin
    gs_int.push(si, 3);
    gs_int.push(si, 9);
    check(gs_int.describe(si) = "<3..9>" and gs_int.top(si) = 9 and gs_int.depth(si) = 2, "generic stack");
    gs_int.pop(si, e);
    check_equal(e, 9, "generic pop");
    gs_op.push(so, op_add);
    check(equal_nocase(gs_op.describe(so), "<op_add>"), "generic opcode stack");
    check_equal(gm_int.min2(3, 4) + gm_int.max2(3, 4) + gm_int.clamp(20, 0, 10), 17, "gm_int");
    check(gm_int.min_of(ia) = 1 and gm_int.max_of(ia) = 9 and not gm_int.is_sorted(ia), "gm_int arr");
    gm_int.sort(ia);
    check_equal(ia(0) * 10 + ia(4), 19, "gm_int sort");
    check_equal(gm_int.index_of_min(ia), 0, "index_of_min");
    check_equal(gm_real.max_of(ra) + gm_real.min_of(ra), 3.0, 1.0e-9, "gm_real");
    check_equal(gm_char.max2('a', 'z') & gm_char.sorted_copy(ca)(0), "za", "gm_char");
    check_equal(gm_time.max2(1 ns, 2 ns), 2 ns, "gm_time");
    check(gm_point.min_of(pa) = (1, 2), "gm_point");
    check_equal(ga_int_str.lookup(tab, 2, "none"), "two ", "assoc hit");
    check_equal(ga_int_str.lookup(tab, 7, "none"), "none", "assoc miss");
    check(ga_int_str.has_key(tab, 1) and ga_int_str.key_count(tab, 1) = 1, "assoc keys");
    dl := gd_int.shift(gd_int.shift(dl, 7), 8);
    check_equal(gd_int.newest(dl) * 10 + gd_int.oldest(dl), 79, "delay line");
    check_equal(gr_sum.reduce((1, 2, 3, 4)), 10, "reduce sum");
    check_equal(gr_max.reduce((4, 9, 2)) + gr_sum.scan_last((1, 2, 3, 4), 2), 12, "reduce max");
  end procedure;

  procedure t_alias_attr is
  begin
    check_equal(to_s(5) & bool_s(true) & real_s(2.5, 1) & trimmed("  q "), "5true+2.5q", "alias functions");
    check_equal(sum_halves, -160, "alias slices");
    check_equal(lut_all(7) + lut_high(5), 70, "alias elements");
    check_equal(nibble_swap(x"A5"), x"5A", "nibble_swap");
    check_equal(high_nibble(x"A5"), std_logic_vector'("1010"), "high_nibble");
    check(vers = "biglib-1.4" and magic_number = 16#BEEF#, "deferred constants");
    check_equal(describe, "count/count16/operating mode", "attributes");
    check_equal(table_weight, 126, "attribute values");
    check_equal(scale_gain(5) + scale_gain(int_vec'(1, 1, 1, 1, 1)), 18, "scale_gain");
    check_equal(sev_labels(error), "error  ", "severity labels");
    check_equal(origin.x + ten, 10, "constants");
  end procedure;

begin
  p_clk : process
  begin
    clock_gen(clk, 10 ns, 25);
    wait;
  end process p_clk;

  u_pipe : entity work.bl_pipe generic map (width => 8, stages => 3) port map (clk => clk, d => d, q => q);
  g_sel : for k in 0 to 3 generate
    u_sel : entity work.bl_sel generic map (kind => k) port map (a => sa, b => sb - 2, y => y(k));
  end generate g_sel;
  u_sc1 : entity work.bl_scale port map (x => sa, y => sc1);
  u_sc2 : entity work.bl_scale generic map (shift_by => 5, saturate => true) port map (x => sa, y => sc2);
  u_sc3 : entity work.bl_scale generic map (shift_by => 2) port map (x => sa, y => sc3);
  u_gate : entity work.bl_gate port map (en => gate_en, d => x"9", q => gate_q);

  p_strobe : process
  begin
    wait for 50 ns;
    pulse(strobe, 15 ns);
    wait;
  end process p_strobe;

  p_mon : process
    variable s : int_vec(0 to 11);
  begin
    wait for 85 ns;
    sample_n(clk, q, s, 12);
    for i in s'range loop
      if s(i) /= 0 then
        mon_q.push(s(i));
      end if;
    end loop;
    mon_done <= true;
    wait;
  end process p_mon;

  p_assert : process (all)
  begin
    assert sa = sb or sa /= sb report "unreachable" severity failure;
  end process p_assert;

  p_main : process
    variable ok : boolean;
    variable edges : natural;
    variable seen : int_vec(0 to 4);
  begin
    t_chars;
    t_strings;
    t_conv;
    t_vec;
    t_records;
    t_time_math;
    t_bits;
    t_calendar_enums;
    t_text;
    t_linked;
    t_prot;
    t_generics;
    t_alias_attr;
    wait_high(strobe, 200 ns, ok);
    check(ok, "strobe seen");
    check_equal(now, 50 ns, "strobe time");
    count_edges(strobe, 40 ns, edges);
    check_equal(edges, 1, "strobe edges");
    wait for 2 ns;
    drive_seq(clk, d, (11, 22, 33, 44, 55, 0, 0, 0, 0, 0));
    wait until mon_done;
    for i in seen'range loop
      seen(i) := mon_q.pop;
    end loop;
    check_equal(seen, int_vec'(11, 22, 33, 44, 55), "pipeline contents");
    check_equal(to_integer(y(0)) * 1000 + to_integer(y(1)) * 100 + to_integer(y(2)) * 10 + to_integer(y(3)), 22000 + 200 + 80 + 14, "sel kinds");
    check_equal(to_integer(sc1) + to_integer(sc2) + to_integer(sc3), 12 + 255 + 48, "scale variants");
    gate_en <= '1';
    wait for 1 ns;
    check_equal(gate_q, x"9", "guarded block");
    summary("big_lib");
    report "PASS" severity note;
    wait;
  end process p_main;
end architecture tb;
