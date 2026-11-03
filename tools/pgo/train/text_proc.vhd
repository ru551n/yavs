-- pgo: top=text_proc_tb std=2008 stop=2ms
-- Reproduces text-heavy testbench code: building and parsing lines with textio (write, hwrite,
-- owrite, bwrite and the matching reads), to_string family, string helpers, look-up tables of
-- strings, composed report messages and severity levels, plus a clocked monitor that formats
-- and re-parses a value every cycle.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

package text_util is
  type string_vec_t is array (natural range <>) of string;
  type sev_names_t is array (severity_level) of string(1 to 7);

  constant month_names : string_vec_t(1 to 12)(1 to 3) :=
    ("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec");
  constant day_names : string_vec_t(0 to 6)(1 to 9) :=
    ("Monday   ", "Tuesday  ", "Wednesday", "Thursday ", "Friday   ", "Saturday ", "Sunday   ");
  constant mnemonics : string_vec_t(0 to 15)(1 to 4) :=
    ("NOP ", "LDA ", "STA ", "ADD ", "SUB ", "AND ", "OR  ", "XOR ",
     "SHL ", "SHR ", "JMP ", "JZ  ", "JNZ ", "CALL", "RET ", "HALT");
  constant csv_rows : string_vec_t(0 to 5)(1 to 35) :=
    ("10, 20 ,x=0x1F, -7                 ",
     "  1,2,3,4,5,6,7,8,9,10             ",
     "alpha, beta=12, gamma , 0xFF       ",
     "-100, 250, y=0x80, 3               ",
     "name=text_proc, depth=16, width=8  ",
     "   99    98    97   x=0x10         ");
  constant sev_names : sev_names_t := ("NOTE   ", "WARNING", "ERROR  ", "FAILURE");

  function to_upper (s : string) return string;
  function to_lower (s : string) return string;
  function equal_ci (a, b : string) return boolean;
  function trim (s : string) return string;
  function is_palindrome (s : string) return boolean;
  function rot13 (s : string) return string;
  function count_words (s : string) return natural;
  function rle (s : string) return string;
  function itoa (v : integer; base : positive range 2 to 16 := 10; width : natural := 0) return string;
  function atoi (s : string; base : positive range 2 to 16 := 10) return integer;
  function roman (n : positive) return string;
  function center (s : string; width : natural) return string;
  function find_char_eq (s : string) return natural;
  function find_name (tab : string_vec_t; key : string) return integer;
  procedure skip_blanks (l : inout line);
  procedure next_token (l : inout line; tok : out string; len : out natural);
end package text_util;

package body text_util is
  function to_upper (s : string) return string is
    variable r : string(s'range) := s;
  begin
    for i in r'range loop
      if r(i) >= 'a' and r(i) <= 'z' then
        r(i) := character'val(character'pos(r(i)) - 32);
      end if;
    end loop;
    return r;
  end function;

  function to_lower (s : string) return string is
    variable r : string(s'range) := s;
  begin
    for i in r'range loop
      if r(i) >= 'A' and r(i) <= 'Z' then
        r(i) := character'val(character'pos(r(i)) + 32);
      end if;
    end loop;
    return r;
  end function;

  function equal_ci (a, b : string) return boolean is
  begin
    return to_lower(a) = to_lower(b);
  end function;

  function trim (s : string) return string is
    variable lo : integer := s'left;
    variable hi : integer := s'right;
  begin
    while lo <= hi and s(lo) = ' ' loop
      lo := lo + 1;
    end loop;
    while hi >= lo and s(hi) = ' ' loop
      hi := hi - 1;
    end loop;
    return s(lo to hi);
  end function;

  function is_palindrome (s : string) return boolean is
    variable i : integer := s'left;
    variable j : integer := s'right;
  begin
    while i < j loop
      if to_lower(s(i to i)) /= to_lower(s(j to j)) then
        return false;
      end if;
      i := i + 1;
      j := j - 1;
    end loop;
    return true;
  end function;

  function rot13 (s : string) return string is
    variable r : string(1 to s'length) := s;
    variable p : natural;
  begin
    for i in r'range loop
      p := character'pos(r(i));
      if r(i) >= 'a' and r(i) <= 'z' then
        r(i) := character'val((p - 97 + 13) mod 26 + 97);
      elsif r(i) >= 'A' and r(i) <= 'Z' then
        r(i) := character'val((p - 65 + 13) mod 26 + 65);
      end if;
    end loop;
    return r;
  end function;

  function count_words (s : string) return natural is
    variable n : natural := 0;
    variable inside : boolean := false;
  begin
    for i in s'range loop
      if s(i) /= ' ' and not inside then
        n := n + 1;
        inside := true;
      elsif s(i) = ' ' then
        inside := false;
      end if;
    end loop;
    return n;
  end function;

  -- Run length encoding: "aaab" becomes "a3b1".
  function rle (s : string) return string is
    variable buf : string(1 to 128);
    variable n : natural := 0;
    variable run : natural;
    variable i : integer := s'left;
  begin
    while i <= s'right loop
      run := 1;
      while i + run <= s'right and s(i + run) = s(i) loop
        run := run + 1;
      end loop;
      n := n + 1;
      buf(n) := s(i);
      buf(n + 1 to n + integer'image(run)'length) := integer'image(run);
      n := n + integer'image(run)'length;
      i := i + run;
    end loop;
    return buf(1 to n);
  end function;

  function itoa (v : integer; base : positive range 2 to 16 := 10; width : natural := 0) return string is
    constant digits : string(1 to 16) := "0123456789abcdef";
    variable tmp : string(1 to 40);
    variable n : natural := 0;
    variable x : natural := abs v;
    variable c : character;
  begin
    loop
      n := n + 1;
      tmp(n) := digits(x mod base + 1);
      x := x / base;
      exit when x = 0;
    end loop;
    while n < width loop
      n := n + 1;
      tmp(n) := '0';
    end loop;
    if v < 0 then
      n := n + 1;
      tmp(n) := '-';
    end if;
    for i in 1 to n / 2 loop
      c := tmp(i);
      tmp(i) := tmp(n - i + 1);
      tmp(n - i + 1) := c;
    end loop;
    return tmp(1 to n);
  end function;

  function atoi (s : string; base : positive range 2 to 16 := 10) return integer is
    variable r : integer := 0;
    variable d : integer;
    variable neg : boolean := false;
    variable c : character;
  begin
    for i in s'range loop
      c := s(i);
      if c = '-' then
        neg := true;
      else
        case c is
          when '0' to '9' => d := character'pos(c) - 48;
          when 'a' to 'f' => d := character'pos(c) - 87;
          when 'A' to 'F' => d := character'pos(c) - 55;
          when others => d := 99;
        end case;
        assert d < base report "atoi: bad digit '" & c & "' in " & s severity failure;
        r := r * base + d;
      end if;
    end loop;
    if neg then
      return -r;
    end if;
    return r;
  end function;

  function roman (n : positive) return string is
    type val_arr_t is array (0 to 12) of natural;
    constant vals : val_arr_t := (1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1);
    constant syms : string_vec_t(0 to 12)(1 to 2) :=
      ("M ", "CM", "D ", "CD", "C ", "XC", "L ", "XL", "X ", "IX", "V ", "IV", "I ");
    variable buf : string(1 to 32);
    variable len : natural := 0;
    variable x : natural := n;
  begin
    for i in 0 to 12 loop
      while x >= vals(i) loop
        buf(len + 1 to len + trim(syms(i))'length) := trim(syms(i));
        len := len + trim(syms(i))'length;
        x := x - vals(i);
      end loop;
    end loop;
    return trim(buf(1 to len));
  end function;

  function center (s : string; width : natural) return string is
    constant pad : integer := width - s'length;
  begin
    if pad <= 0 then
      return s;
    end if;
    return (1 to pad / 2 => ' ') & s & (1 to pad - pad / 2 => ' ');
  end function;

  function find_char_eq (s : string) return natural is
  begin
    for i in s'range loop
      if s(i) = '=' then
        return i - s'left + 1;
      end if;
    end loop;
    return 0;
  end function;

  function find_name (tab : string_vec_t; key : string) return integer is
  begin
    for i in tab'range loop
      if equal_ci(trim(tab(i)), trim(key)) then
        return i;
      end if;
    end loop;
    return -1;
  end function;

  procedure skip_blanks (l : inout line) is
    variable c : character;
    variable good : boolean;
  begin
    while l /= null and l'length > 0 and (l(l'left) = ' ' or l(l'left) = ',') loop
      read(l, c, good);
    end loop;
  end procedure;

  procedure next_token (l : inout line; tok : out string; len : out natural) is
    variable c : character;
    variable good : boolean;
    variable n : natural := 0;
  begin
    skip_blanks(l);
    while l /= null and l'length > 0 loop
      exit when l(l'left) = ' ' or l(l'left) = ',';
      read(l, c, good);
      exit when not good;
      n := n + 1;
      if n <= tok'length then
        tok(tok'left + n - 1) := c;
      end if;
    end loop;
    len := minimum(n, tok'length);
  end procedure;
end package body text_util;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
use work.text_util.all;

entity text_proc_tb is
end entity text_proc_tb;

architecture tb of text_proc_tb is
  signal clk : std_logic := '0';
  signal finished : boolean := false;
  signal lfsr : std_logic_vector(15 downto 0) := x"ace1";
  signal mon_count : natural := 0;
  signal mon_done : boolean := false;
  constant mon_cycles : natural := 9000;
begin
  clk <= not clk after 5 ns when not finished else clk;

  -- 16 bit LFSR as stimulus.
  gen : process (clk)
  begin
    if rising_edge(clk) then
      lfsr <= lfsr(14 downto 0) & (lfsr(15) xor lfsr(13) xor lfsr(12) xor lfsr(10));
    end if;
  end process;

  -- Monitor: format the sampled value in three radixes, parse it back, compare.
  monitor : process
    variable l : line;
    variable v, back, bback : std_logic_vector(15 downto 0);
    variable oback : std_logic_vector(14 downto 0);
    variable pre : string(1 to 5);
    variable n : integer;
    variable good : boolean;
    variable sum : natural := 0;
  begin
    for cyc in 1 to mon_cycles loop
      wait until rising_edge(clk);
      v := lfsr;
      write(l, string'(" hex="));
      hwrite(l, v);
      write(l, string'(" oct="));
      owrite(l, v(14 downto 0));
      write(l, string'(" bin="));
      bwrite(l, v);
      write(l, string'(" dec="));
      write(l, to_integer(unsigned(v)));
      read(l, pre, good);
      hread(l, back, good);
      assert good and back = v report "hex round trip " & to_hstring(back) & " /= " & to_hstring(v) severity failure;
      read(l, pre, good);
      oread(l, oback, good);
      assert good and oback = v(14 downto 0) report "octal round trip" severity failure;
      read(l, pre, good);
      bread(l, bback, good);
      assert good and bback = v report "binary round trip" severity failure;
      read(l, pre, good);
      read(l, n, good);
      assert good and n = to_integer(unsigned(v)) report "decimal round trip" severity failure;
      deallocate(l);
      sum := sum + to_integer(unsigned(back(7 downto 0)));
      mon_count <= cyc;
    end loop;
    assert sum > 0 report "monitor saw nothing" severity failure;
    mon_done <= true;
    wait;
  end process;

  main : process
    variable l, m : line;
    variable slv16 : std_logic_vector(15 downto 0);
    variable slv10 : std_logic_vector(9 downto 0);
    variable bv : bit_vector(7 downto 0);
    variable i, j, total : integer;
    variable r : real;
    variable good : boolean;
    variable tok : string(1 to 32);
    variable tlen : natural;
    variable key : line;
    variable acc : integer;
    variable words : natural;
    variable s : string(1 to 12);
    variable o16 : std_logic_vector(15 downto 0);

    function parse_num (t : string) return integer is
    begin
      if t'length > 2 and t(t'left to t'left + 1) = "0x" then
        return atoi(t(t'left + 2 to t'right), 16);
      end if;
      return atoi(t);
    end function;

    function is_num (t : string) return boolean is
    begin
      return t'length > 0 and ((t(t'left) >= '0' and t(t'left) <= '9') or t(t'left) = '-');
    end function;

    procedure expect (cond : boolean; msg : string) is
    begin
      assert cond report "check failed: " & msg severity failure;
    end procedure;

    procedure expect_line (l : inout line; want : string; msg : string) is
    begin
      expect(l.all = want, msg & ": '" & l.all & "' /= '" & want & "'");
      deallocate(l);
    end procedure;

    procedure test_write is
    begin
      write(l, 42, right, 6);
      expect_line(l, "    42", "write right");
      write(l, string'("abc"), left, 6);
      expect_line(l, "abc   ", "write left");
      write(l, -17, left, 5);
      expect_line(l, "-17  ", "negative left");
      write(l, 3.14159, right, 0, 3);
      expect_line(l, "3.142", "real digits");
      write(l, 12 ns, right, 0, ns);
      expect_line(l, "12 ns", "time unit");
      write(l, 2500 ps, left, 0, ns);
      expect_line(l, "2.5 ns", "time fractional");
      write(l, character'('x'));
      write(l, string'(" = "));
      write(l, true);
      expect(equal_ci(l.all, "x = true"), "boolean write");
      deallocate(l);
      hwrite(l, std_logic_vector'(x"BEEF"));
      expect_line(l, "BEEF", "hwrite");
      slv10 := "1010100101";
      hwrite(l, slv10);
      expect_line(l, "2A5", "hwrite padded");
      owrite(l, std_logic_vector'("101010"));
      expect_line(l, "52", "owrite");
      bwrite(l, std_logic_vector'("1X0Z"));
      expect_line(l, "1X0Z", "bwrite unknown");
      bv := "10100101";
      write(l, bv);
      expect_line(l, "10100101", "write bit_vector");
    end procedure;

    procedure test_read is
      variable n : integer;
      variable c : character;
      variable f : real;
      variable t : time;
      variable txt : string(1 to 5);
    begin
      l := new string'("  123 -45 abc  7.25 15 ns FF 777 1011");
      read(l, n, good);
      expect(good and n = 123, "read integer");
      read(l, n, good);
      expect(good and n = -45, "read negative");
      read(l, txt(1 to 3), good);
      expect(good, "read fixed string");
      expect(txt(1 to 3) = " ab", "fixed string content");
      read(l, c, good);
      expect(good and c = 'c', "read char");
      read(l, f, good);
      expect(good and f > 7.249 and f < 7.251, "read real");
      read(l, t, good);
      expect(good and t = 15 ns, "read time");
      hread(l, slv16(7 downto 0), good);
      expect(good and slv16(7 downto 0) = x"FF", "hread");
      oread(l, slv16(8 downto 0), good);
      expect(good and slv16(8 downto 0) = "111111111", "oread");
      bread(l, slv16(3 downto 0), good);
      expect(good and slv16(3 downto 0) = "1011", "bread");
      read(l, n, good);
      expect(not good, "read past end");
      deallocate(l);
      l := new string'("12x");
      read(l, n, good);
      expect(good and n = 12 and l.all = "x", "read stops at letter");
      deallocate(l);
    end procedure;

    procedure test_round_trips is
    begin
      for k in 0 to 5999 loop
        slv16 := std_logic_vector(to_unsigned((k * 2477) mod 65536, 16));
        hwrite(l, slv16);
        write(l, string'(" "));
        owrite(l, slv16);
        write(l, string'(" "));
        bwrite(l, slv16);
        write(l, string'(" "));
        write(l, k - 1500);
        hread(l, slv16, good);
        expect(good and slv16 = std_logic_vector(to_unsigned((k * 2477) mod 65536, 16)), "hex trip");
        oread(l, o16, good);
        expect(good and o16 = std_logic_vector(to_unsigned((k * 2477) mod 65536, 16)), "octal trip");
        bread(l, slv16, good);
        expect(good and slv16 = o16, "binary trip");
        read(l, j, good);
        expect(good and j = k - 1500, "int trip");
        deallocate(l);
        expect(atoi(itoa(k - 1500, 10), 10) = k - 1500, "itoa 10");
        expect(atoi(itoa(k, 16), 16) = k, "itoa 16");
        expect(atoi(itoa(k, 2, 16), 2) = k, "itoa 2");
      end loop;
    end procedure;

    procedure test_to_string is
    begin
      expect(to_string(42) = "42", "to_string integer");
      expect(to_string(-5) = "-5", "to_string negative");
      expect(to_hstring(std_logic_vector'(x"1f")) = "1F", "to_hstring");
      expect(to_bstring(std_logic_vector'("1010")) = "1010", "to_bstring");
      expect(to_ostring(std_logic_vector'("111001")) = "71", "to_ostring");
      expect(to_string(10 ns, ns) = "10 ns", "to_string time");
      expect(equal_ci(to_string(true), "true"), "to_string boolean");
      expect(to_string('q') = "q", "to_string char");
      expect(to_string(2.5, 2) = "2.50", "to_string real");
      expect(to_string(bit_vector'("0110")) = "0110", "to_string bit_vector");
      expect(equal_ci(severity_level'image(warning), "warning"), "severity image");
      expect(sev_names(error) = "ERROR  ", "severity table");
    end procedure;

    procedure test_helpers is
    begin
      expect(to_upper("Hello World 9") = "HELLO WORLD 9", "upper");
      expect(to_lower("Hello World 9") = "hello world 9", "lower");
      expect(equal_ci("VHDL", "vhdl"), "equal_ci");
      expect(not equal_ci("VHDL", "vhd"), "equal_ci length");
      expect(trim("   padded text   ") = "padded text", "trim");
      expect(is_palindrome("Racecar"), "palindrome");
      expect(not is_palindrome("Racecars"), "not palindrome");
      expect(rot13(rot13("Round Trip!")) = "Round Trip!", "rot13 twice");
      expect(rot13("Hello") = "Uryyb", "rot13");
      expect(count_words("  one two   three ") = 3, "count_words");
      expect(rle("aaabccdddd") = "a3b1c2d4", "rle");
      expect(roman(1994) = "MCMXCIV", "roman 1994");
      expect(roman(49) = "XLIX", "roman 49");
      expect(center("ab", 6) = "  ab  ", "center");
      expect(itoa(255, 16) = "ff", "itoa hex");
      expect(itoa(-5, 2, 6) = "-000101", "itoa binary padded");
      expect(atoi("-ff", 16) = -255, "atoi hex");
      for n in 1 to 400 loop
        expect(roman(n)'length >= 1, "roman length");
      end loop;
    end procedure;

    -- Parse every table row into numbers (decimal and 0x hex), keys and plain words.
    procedure test_parse is
    begin
      acc := 0;
      words := 0;
      for row in csv_rows'range loop
        l := new string'(trim(csv_rows(row)));
        loop
          next_token(l, tok, tlen);
          exit when tlen = 0;
          if is_num(tok(1 to tlen)) then
            acc := acc + parse_num(tok(1 to tlen));
          else
            j := find_char_eq(tok(1 to tlen));
            if j > 0 and is_num(tok(j + 1 to tlen)) then
              acc := acc + parse_num(tok(j + 1 to tlen));
            else
              words := words + 1;
            end if;
          end if;
        end loop;
        deallocate(l);
      end loop;
      expect(acc = 991, "parse sum " & integer'image(acc));
      expect(words = 3, "parse words " & integer'image(words));
    end procedure;

    procedure test_tables is
    begin
      for k in month_names'range loop
        expect(find_name(month_names, to_upper(month_names(k))) = k, "month lookup");
      end loop;
      for k in mnemonics'range loop
        expect(find_name(mnemonics, to_lower(trim(mnemonics(k)))) = k, "mnemonic lookup");
      end loop;
      expect(find_name(day_names, "sunday") = 6, "day lookup");
      expect(find_name(day_names, "someday") = -1, "day miss");
    end procedure;

    -- Print a few tables and composed messages.
    procedure test_reports is
    begin
      for k in mnemonics'range loop
        write(l, string'("op "));
        hwrite(l, std_logic_vector(to_unsigned(k, 4)));
        write(l, string'(" "));
        write(l, trim(mnemonics(k)), left, 5);
        bwrite(l, std_logic_vector(to_unsigned(k, 4)));
        write(l, string'(" "));
        write(l, roman(k + 1), right, 5);
        writeline(output, l);
      end loop;
      for k in 1 to 12 loop
        report month_names(k) & ": " & integer'image(k) & " day " & trim(day_names(k mod 7)) &
               " " & to_hstring(std_logic_vector(to_unsigned(k * 17, 8)))
               severity note;
      end loop;
      assert true report "unreachable note" severity note;
      assert true report "unreachable warning" severity warning;
      assert true report "unreachable error" severity error;
      assert mon_count >= 0 report "unreachable failure" severity failure;
      assert false report "assert at note level fires on purpose" severity note;
    end procedure;

    -- String building in loops.
    procedure test_concat is
      variable acc_s : line := new string'("");
      variable tmp : line;
    begin
      for k in 1 to 300 loop
        tmp := new string'(acc_s.all & integer'image(k mod 10));
        deallocate(acc_s);
        acc_s := tmp;
      end loop;
      expect(acc_s'length = 300, "concat length");
      expect(acc_s(1 to 12) = "123456789012", "concat head");
      for k in 1 to 200 loop
        s := to_hstring(std_logic_vector(to_unsigned(k * 3001, 24))) & "abcdef";
        expect(s(1 to 6) = to_hstring(std_logic_vector(to_unsigned(k * 3001, 24))), "hex slice");
        words := count_words(rle(s) & " " & rot13(s));
        expect(words = 2, "words after concat");
      end loop;
      deallocate(acc_s);
    end procedure;
  begin
    test_write;
    test_read;
    test_to_string;
    test_helpers;
    test_parse;
    test_tables;
    test_round_trips;
    test_concat;
    test_reports;
    wait until mon_done;
    expect(mon_count = mon_cycles, "monitor cycles");
    report "PASS" severity note;
    finished <= true;
    wait;
  end process;
end architecture tb;
