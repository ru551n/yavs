-- pgo: top=mini_framework_tb std=2008 stop=400us analyze=6
-- Reproduces a small unit testbench built on a test framework: a large closure of packages
-- (strings, conversions, log, queue, generic stack, checks, runner), a context, an entity with
-- several architectures plus a configuration, then a short run of four test cases.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Basic types and constants.
package fw_types is
  type log_level_t is (lvl_debug, lvl_info, lvl_warning, lvl_error, lvl_fatal);
  type handler_kind_t is (h_console, h_memory, h_counter, h_none);
  type verdict_t is (v_pending, v_pass, v_fail, v_skipped);
  subtype nibble_t is natural range 0 to 15;
  subtype byte_t is natural range 0 to 255;
  subtype word_t is std_logic_vector(31 downto 0);
  subtype byte_slv_t is std_logic_vector(7 downto 0);
  type int_vec_t is array (natural range <>) of integer;
  type bool_vec_t is array (natural range <>) of boolean;
  type slv_vec_t is array (natural range <>) of std_logic_vector;
  type str_ptr_t is access string;
  type str_vec_t is array (natural range <>) of str_ptr_t;
  type counts_t is array (log_level_t) of natural;
  type handler_map_t is array (log_level_t) of handler_kind_t;
  type check_stat_t is record
    passed : natural;
    failed : natural;
    last_id : integer;
    verdict : verdict_t;
  end record;
  constant hex_digits : string(1 to 16) := "0123456789ABCDEF";
  constant max_mem_lines : positive := 32;
  constant empty_stat : check_stat_t := (0, 0, -1, v_pending);
  constant default_handlers : handler_map_t :=
    (lvl_debug => h_none, lvl_info => h_console, lvl_warning => h_memory,
     lvl_error => h_memory, lvl_fatal => h_console);
end package fw_types;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.fw_types.all;

-- String helpers.
package fw_strings is
  function upper (s : string) return string;
  function lower (s : string) return string;
  function strip (s : string) return string;
  function starts_with (s, prefix : string) return boolean;
  function ends_with (s, suffix : string) return boolean;
  function find_char (s : string; c : character; from : positive := 1) return natural;
  function count_char (s : string; c : character) return natural;
  function replace (s : string; from_c, to_c : character) return string;
  function replace (s, old_s, new_s : string) return string;
  function split_count (s : string; sep : character) return natural;
  function split_nth (s : string; sep : character; n : natural) return string;
  function parse_int (s : string) return integer;
  function is_digits (s : string) return boolean;
  function pad_left (s : string; width : natural; fill : character := ' ') return string;
  function repeat_str (s : string; n : natural) return string;
  function to_hex (v : natural; digits : positive := 1) return string;
  function to_hex (v : std_logic_vector) return string;
  function int_img (i : integer) return string;
  function level_tag (l : log_level_t) return string;
  function to_string (v : int_vec_t) return string;
  function join (v : int_vec_t; sep : string := ",") return string;
end package fw_strings;

package body fw_strings is
  function upper (s : string) return string is
    variable r : string(1 to s'length) := s;
  begin
    for i in r'range loop
      if r(i) >= 'a' and r(i) <= 'z' then
        r(i) := character'val(character'pos(r(i)) - 32);
      end if;
    end loop;
    return r;
  end function;

  function lower (s : string) return string is
    variable r : string(1 to s'length) := s;
  begin
    for i in r'range loop
      if r(i) >= 'A' and r(i) <= 'Z' then
        r(i) := character'val(character'pos(r(i)) + 32);
      end if;
    end loop;
    return r;
  end function;

  function strip (s : string) return string is
    variable lo : integer := s'left;
    variable hi : integer := s'right;
  begin
    while lo <= hi and (s(lo) = ' ' or s(lo) = HT) loop
      lo := lo + 1;
    end loop;
    while hi >= lo and (s(hi) = ' ' or s(hi) = HT) loop
      hi := hi - 1;
    end loop;
    if hi < lo then
      return "";
    end if;
    return s(lo to hi);
  end function;

  function starts_with (s, prefix : string) return boolean is
    constant n : natural := prefix'length;
  begin
    if s'length < n then
      return false;
    end if;
    return s(s'left to s'left + n - 1) = prefix;
  end function;

  function ends_with (s, suffix : string) return boolean is
    constant n : natural := suffix'length;
  begin
    if s'length < n then
      return false;
    end if;
    return s(s'right - n + 1 to s'right) = suffix;
  end function;

  function find_char (s : string; c : character; from : positive := 1) return natural is
  begin
    for i in from to s'length loop
      if s(s'left + i - 1) = c then
        return i;
      end if;
    end loop;
    return 0;
  end function;

  function count_char (s : string; c : character) return natural is
    variable n : natural := 0;
  begin
    for i in s'range loop
      if s(i) = c then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;

  function replace (s : string; from_c, to_c : character) return string is
    variable r : string(1 to s'length) := s;
  begin
    for i in r'range loop
      if r(i) = from_c then
        r(i) := to_c;
      end if;
    end loop;
    return r;
  end function;

  function replace (s, old_s, new_s : string) return string is
    variable buf : string(1 to 256);
    variable n : natural := 0;
    variable i : integer := s'left;
  begin
    while i <= s'right loop
      if old_s'length > 0 and i + old_s'length - 1 <= s'right and
         s(i to i + old_s'length - 1) = old_s then
        buf(n + 1 to n + new_s'length) := new_s;
        n := n + new_s'length;
        i := i + old_s'length;
      else
        n := n + 1;
        buf(n) := s(i);
        i := i + 1;
      end if;
    end loop;
    return buf(1 to n);
  end function;

  function split_count (s : string; sep : character) return natural is
  begin
    if s'length = 0 then
      return 0;
    end if;
    return count_char(s, sep) + 1;
  end function;

  function split_nth (s : string; sep : character; n : natural) return string is
    variable start : integer := s'left;
    variable k : natural := 0;
  begin
    for i in s'range loop
      if s(i) = sep then
        if k = n then
          return strip(s(start to i - 1));
        end if;
        k := k + 1;
        start := i + 1;
      end if;
    end loop;
    if k = n then
      return strip(s(start to s'right));
    end if;
    return "";
  end function;

  function is_digits (s : string) return boolean is
  begin
    if s'length = 0 then
      return false;
    end if;
    for i in s'range loop
      if s(i) < '0' or s(i) > '9' then
        return false;
      end if;
    end loop;
    return true;
  end function;

  function parse_int (s : string) return integer is
    constant t : string := strip(s);
    variable r : integer := 0;
    variable neg : boolean := false;
    variable first : integer := t'left;
  begin
    if t'length > 0 and t(t'left) = '-' then
      neg := true;
      first := first + 1;
    end if;
    for i in first to t'right loop
      assert t(i) >= '0' and t(i) <= '9' report "parse_int: bad digit in " & t severity failure;
      r := r * 10 + (character'pos(t(i)) - character'pos('0'));
    end loop;
    if neg then
      return -r;
    end if;
    return r;
  end function;

  function pad_left (s : string; width : natural; fill : character := ' ') return string is
  begin
    if s'length >= width then
      return s;
    end if;
    return (1 to width - s'length => fill) & s;
  end function;

  function repeat_str (s : string; n : natural) return string is
    variable r : string(1 to s'length * n);
  begin
    for i in 0 to n - 1 loop
      r(i * s'length + 1 to (i + 1) * s'length) := s;
    end loop;
    return r;
  end function;

  function to_hex (v : natural; digits : positive := 1) return string is
    variable n : natural := v;
    variable tmp : string(1 to 12);
    variable r : string(1 to 12);
    variable len : natural := 0;
  begin
    loop
      len := len + 1;
      tmp(len) := hex_digits(n mod 16 + 1);
      n := n / 16;
      exit when n = 0;
    end loop;
    while len < digits loop
      len := len + 1;
      tmp(len) := '0';
    end loop;
    for i in 1 to len loop
      r(i) := tmp(len - i + 1);
    end loop;
    return r(1 to len);
  end function;

  function to_hex (v : std_logic_vector) return string is
    constant nn : natural := (v'length + 3) / 4;
    variable padded : std_logic_vector(nn * 4 - 1 downto 0) := (others => '0');
    variable r : string(1 to nn);
    variable nib : std_logic_vector(3 downto 0);
  begin
    padded(v'length - 1 downto 0) := v;
    for i in 1 to nn loop
      nib := padded((nn - i) * 4 + 3 downto (nn - i) * 4);
      if is_x(nib) then
        r(i) := 'X';
      else
        r(i) := hex_digits(to_integer(unsigned(nib)) + 1);
      end if;
    end loop;
    return r;
  end function;

  function int_img (i : integer) return string is
  begin
    return integer'image(i);
  end function;

  function level_tag (l : log_level_t) return string is
    constant img : string := lower(log_level_t'image(l));
  begin
    return upper(img(img'left + 4 to img'right));
  end function;

  function to_string (v : int_vec_t) return string is
  begin
    return "[" & join(v) & "]";
  end function;

  function join (v : int_vec_t; sep : string := ",") return string is
    variable buf : string(1 to 256);
    variable n : natural := 0;
  begin
    for i in v'range loop
      if i /= v'left then
        buf(n + 1 to n + sep'length) := sep;
        n := n + sep'length;
      end if;
      buf(n + 1 to n + integer'image(v(i))'length) := integer'image(v(i));
      n := n + integer'image(v(i))'length;
    end loop;
    return buf(1 to n);
  end function;
end package body fw_strings;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.fw_types.all;

-- Conversions and bit helpers.
package fw_conv is
  function to_slv (n : natural; width : positive) return std_logic_vector;
  function to_int (v : std_logic_vector) return natural;
  function slv_from_hex (s : string) return std_logic_vector;
  function bin_str (v : std_logic_vector) return string;
  function popcount (v : std_logic_vector) return natural;
  function parity (v : std_logic_vector) return std_logic;
  function reverse_bits (v : std_logic_vector) return std_logic_vector;
  function rotate_left_slv (v : std_logic_vector; n : natural) return std_logic_vector;
  function to_slv_vec (v : int_vec_t; width : positive) return slv_vec_t;
end package fw_conv;

package body fw_conv is
  function to_slv (n : natural; width : positive) return std_logic_vector is
  begin
    return std_logic_vector(to_unsigned(n, width));
  end function;

  function to_int (v : std_logic_vector) return natural is
  begin
    return to_integer(unsigned(v));
  end function;

  function slv_from_hex (s : string) return std_logic_vector is
    variable r : std_logic_vector(s'length * 4 - 1 downto 0);
    variable d : natural;
  begin
    for i in s'range loop
      case s(i) is
        when '0' to '9' => d := character'pos(s(i)) - character'pos('0');
        when 'a' to 'f' => d := character'pos(s(i)) - character'pos('a') + 10;
        when 'A' to 'F' => d := character'pos(s(i)) - character'pos('A') + 10;
        when others => d := 0;
      end case;
      r((s'right - i) * 4 + 3 downto (s'right - i) * 4) := to_slv(d, 4);
    end loop;
    return r;
  end function;

  function bin_str (v : std_logic_vector) return string is
    variable r : string(1 to v'length);
    variable k : natural := 0;
  begin
    for i in v'range loop
      k := k + 1;
      r(k) := std_logic'image(v(i))(2);
    end loop;
    return r;
  end function;

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

  function parity (v : std_logic_vector) return std_logic is
  begin
    if popcount(v) mod 2 = 1 then
      return '1';
    end if;
    return '0';
  end function;

  function reverse_bits (v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'range);
  begin
    for i in v'range loop
      r(v'left + v'right - i) := v(i);
    end loop;
    return r;
  end function;

  function rotate_left_slv (v : std_logic_vector; n : natural) return std_logic_vector is
  begin
    return std_logic_vector(rotate_left(unsigned(v), n));
  end function;

  function to_slv_vec (v : int_vec_t; width : positive) return slv_vec_t is
    variable r : slv_vec_t(v'range)(width - 1 downto 0);
  begin
    for i in v'range loop
      r(i) := to_slv(v(i), width);
    end loop;
    return r;
  end function;
end package body fw_conv;

use work.fw_types.all;

-- Functions that return unconstrained arrays.
package fw_arr is
  function iota (n : natural; start : integer := 0) return int_vec_t;
  function reverse (v : int_vec_t) return int_vec_t;
  function concat (a, b : int_vec_t) return int_vec_t;
  function scale (v : int_vec_t; k : integer) return int_vec_t;
  function sorted (v : int_vec_t) return int_vec_t;
  function sum_of (v : int_vec_t) return integer;
  function max_of (v : int_vec_t) return integer;
  function min_of (v : int_vec_t) return integer;
end package fw_arr;

package body fw_arr is
  function iota (n : natural; start : integer := 0) return int_vec_t is
    variable r : int_vec_t(0 to n - 1);
  begin
    for i in r'range loop
      r(i) := start + i;
    end loop;
    return r;
  end function;

  function reverse (v : int_vec_t) return int_vec_t is
    variable r : int_vec_t(v'range);
  begin
    for i in v'range loop
      r(v'left + v'right - i) := v(i);
    end loop;
    return r;
  end function;

  function concat (a, b : int_vec_t) return int_vec_t is
    variable r : int_vec_t(0 to a'length + b'length - 1);
  begin
    for i in a'range loop
      r(i - a'left) := a(i);
    end loop;
    for i in b'range loop
      r(a'length + i - b'left) := b(i);
    end loop;
    return r;
  end function;

  function scale (v : int_vec_t; k : integer) return int_vec_t is
    variable r : int_vec_t(v'range);
  begin
    for i in v'range loop
      r(i) := v(i) * k;
    end loop;
    return r;
  end function;

  function sorted (v : int_vec_t) return int_vec_t is
    variable r : int_vec_t(0 to v'length - 1) := v;
    variable t : integer;
  begin
    for i in 1 to r'length - 1 loop
      t := r(i);
      for j in i - 1 downto 0 loop
        exit when r(j) <= t;
        r(j + 1) := r(j);
        r(j) := t;
      end loop;
    end loop;
    return r;
  end function;

  function sum_of (v : int_vec_t) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;

  function max_of (v : int_vec_t) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop
      m := maximum(m, v(i));
    end loop;
    return m;
  end function;

  function min_of (v : int_vec_t) return integer is
    variable m : integer := integer'high;
  begin
    for i in v'range loop
      m := minimum(m, v(i));
    end loop;
    return m;
  end function;
end package body fw_arr;

use work.fw_types.all;
use work.fw_strings.all;

-- Logging with levels, a handler per level and a protected state object.
package fw_log is
  type log_state_t is protected
    procedure add (level : log_level_t; msg : string);
    impure function count (level : log_level_t) return natural;
    procedure set_threshold (level : log_level_t);
    impure function threshold return log_level_t;
    procedure set_handler (level : log_level_t; kind : handler_kind_t);
    impure function handler (level : log_level_t) return handler_kind_t;
    impure function mem_count return natural;
    impure function mem_line (i : natural) return string;
    procedure reset;
  end protected;

end package fw_log;

package body fw_log is
  type log_state_t is protected body
    variable counts : counts_t := (others => 0);
    variable handlers : handler_map_t := default_handlers;
    variable thresh : log_level_t := lvl_info;
    variable mem : str_vec_t(0 to max_mem_lines - 1);
    variable mem_n : natural := 0;
    variable total : natural := 0;

    procedure add (level : log_level_t; msg : string) is
      variable slot : natural;
    begin
      counts(level) := counts(level) + 1;
      total := total + 1;
      if level < thresh and handlers(level) /= h_memory then
        return;
      end if;
      case handlers(level) is
        when h_console =>
          report "[" & level_tag(level) & "] " & msg severity note;
        when h_memory =>
          slot := mem_n mod max_mem_lines;
          if mem(slot) /= null then
            deallocate(mem(slot));
          end if;
          mem(slot) := new string'(level_tag(level) & ": " & msg);
          mem_n := mem_n + 1;
        when h_counter | h_none =>
          null;
      end case;
    end procedure;

    impure function count (level : log_level_t) return natural is
    begin
      return counts(level);
    end function;

    procedure set_threshold (level : log_level_t) is
    begin
      thresh := level;
    end procedure;

    impure function threshold return log_level_t is
    begin
      return thresh;
    end function;

    procedure set_handler (level : log_level_t; kind : handler_kind_t) is
    begin
      handlers(level) := kind;
    end procedure;

    impure function handler (level : log_level_t) return handler_kind_t is
    begin
      return handlers(level);
    end function;

    impure function mem_count return natural is
    begin
      return mem_n;
    end function;

    impure function mem_line (i : natural) return string is
    begin
      if mem(i mod max_mem_lines) = null then
        return "";
      end if;
      return mem(i mod max_mem_lines).all;
    end function;

    procedure reset is
    begin
      counts := (others => 0);
      mem_n := 0;
      total := 0;
    end procedure;
  end protected body;

end package body fw_log;

use work.fw_types.all;
use work.fw_log.all;

-- The shared log object and the shortcut procedures.
package fw_logger is
  shared variable the_log : log_state_t;
  procedure log_debug (msg : string);
  procedure log_info (msg : string);
  procedure log_warning (msg : string);
  procedure log_error (msg : string);
end package fw_logger;

package body fw_logger is
  procedure log_debug (msg : string) is
  begin
    the_log.add(lvl_debug, msg);
  end procedure;

  procedure log_info (msg : string) is
  begin
    the_log.add(lvl_info, msg);
  end procedure;

  procedure log_warning (msg : string) is
  begin
    the_log.add(lvl_warning, msg);
  end procedure;

  procedure log_error (msg : string) is
  begin
    the_log.add(lvl_error, msg);
  end procedure;
end package body fw_logger;

use work.fw_types.all;

-- A queue held in a growing access array.
package fw_queue is
  type int_queue_t is protected
    procedure push (v : integer);
    impure function pop return integer;
    impure function peek return integer;
    impure function size return natural;
    impure function capacity return natural;
    impure function empty return boolean;
    procedure clear;
  end protected;
end package fw_queue;

package body fw_queue is
  type int_queue_t is protected body
    type int_arr_ptr_t is access int_vec_t;
    variable data : int_arr_ptr_t := new int_vec_t(0 to 3);
    variable head : natural := 0;
    variable n : natural := 0;

    procedure push (v : integer) is
      variable bigger : int_arr_ptr_t;
    begin
      if n = data'length then
        bigger := new int_vec_t(0 to 2 * data'length - 1);
        for i in 0 to n - 1 loop
          bigger(i) := data((head + i) mod data'length);
        end loop;
        deallocate(data);
        data := bigger;
        head := 0;
      end if;
      data((head + n) mod data'length) := v;
      n := n + 1;
    end procedure;

    impure function pop return integer is
      variable v : integer;
    begin
      assert n > 0 report "pop from empty queue" severity failure;
      v := data(head);
      head := (head + 1) mod data'length;
      n := n - 1;
      return v;
    end function;

    impure function peek return integer is
    begin
      return data(head);
    end function;

    impure function size return natural is
    begin
      return n;
    end function;

    impure function capacity return natural is
    begin
      return data'length;
    end function;

    impure function empty return boolean is
    begin
      return n = 0;
    end function;

    procedure clear is
    begin
      head := 0;
      n := 0;
    end procedure;
  end protected body;
end package body fw_queue;

-- Generic package with a type, a default value and an image function.
package fw_gen_stack is
  generic (
    type elem_t;
    default_elem : elem_t;
    function img (e : elem_t) return string
  );
  type elem_vec_t is array (natural range <>) of elem_t;
  type stack_t is protected
    procedure push (e : elem_t);
    impure function pop return elem_t;
    impure function top return elem_t;
    impure function depth return natural;
    impure function dump return string;
  end protected;
  function make_vec (n : natural; e : elem_t) return elem_vec_t;
end package fw_gen_stack;

package body fw_gen_stack is
  type stack_t is protected body
    type vec_ptr_t is access elem_vec_t;
    variable items : vec_ptr_t := new elem_vec_t(0 to 7);
    variable n : natural := 0;

    procedure push (e : elem_t) is
      variable bigger : vec_ptr_t;
    begin
      if n = items'length then
        bigger := new elem_vec_t(0 to 2 * items'length - 1);
        bigger(0 to n - 1) := items(0 to n - 1);
        deallocate(items);
        items := bigger;
      end if;
      items(n) := e;
      n := n + 1;
    end procedure;

    impure function pop return elem_t is
    begin
      if n = 0 then
        return default_elem;
      end if;
      n := n - 1;
      return items(n);
    end function;

    impure function top return elem_t is
    begin
      if n = 0 then
        return default_elem;
      end if;
      return items(n - 1);
    end function;

    impure function depth return natural is
    begin
      return n;
    end function;

    impure function dump return string is
      variable buf : string(1 to 200);
      variable len : natural := 0;
    begin
      for i in 0 to n - 1 loop
        if i > 0 then
          len := len + 1;
          buf(len) := ' ';
        end if;
        buf(len + 1 to len + img(items(i))'length) := img(items(i));
        len := len + img(items(i))'length;
      end loop;
      return buf(1 to len);
    end function;
  end protected body;

  function make_vec (n : natural; e : elem_t) return elem_vec_t is
    variable r : elem_vec_t(0 to n - 1) := (others => e);
  begin
    return r;
  end function;
end package body fw_gen_stack;

use work.fw_types.all;
use work.fw_strings.all;

-- Instance of the generic stack for integers.
package int_stack_pkg is new work.fw_gen_stack
  generic map (elem_t => integer, default_elem => -1, img => int_img);

library ieee;
use ieee.std_logic_1164.all;
use work.fw_types.all;
use work.fw_logger.all;
use work.fw_strings.all;
use work.fw_conv.all;

-- Checks that count results and log failures.
package fw_check is
  procedure check_true (cond : boolean; msg : string := "");
  procedure check_equal (got, expected : integer; msg : string := "");
  procedure check_equal (got, expected : string; msg : string := "");
  procedure check_equal (got, expected : std_logic_vector; msg : string := "");
  procedure check_equal (got, expected : boolean; msg : string := "");
  procedure check_near (got, expected, tol : integer; msg : string := "");
  procedure begin_expect_failures;
  impure function end_expect_failures return natural;
  impure function passed_count return natural;
  impure function failed_count return natural;
end package fw_check;

package body fw_check is
  type check_stats_t is protected
    procedure note (ok : boolean);
    impure function expecting return boolean;
    procedure begin_expect;
    impure function end_expect return natural;
    impure function passed return natural;
    impure function failed return natural;
  end protected;

  type check_stats_t is protected body
    variable stat : check_stat_t := empty_stat;
    variable expect_depth : natural := 0;
    variable expected_fails : natural := 0;

    procedure note (ok : boolean) is
    begin
      if ok then
        stat.passed := stat.passed + 1;
      elsif expect_depth > 0 then
        expected_fails := expected_fails + 1;
      else
        stat.failed := stat.failed + 1;
        stat.verdict := v_fail;
      end if;
    end procedure;

    impure function expecting return boolean is
    begin
      return expect_depth > 0;
    end function;

    procedure begin_expect is
    begin
      expect_depth := expect_depth + 1;
      expected_fails := 0;
    end procedure;

    impure function end_expect return natural is
    begin
      expect_depth := expect_depth - 1;
      return expected_fails;
    end function;

    impure function passed return natural is
    begin
      return stat.passed;
    end function;

    impure function failed return natural is
    begin
      return stat.failed;
    end function;
  end protected body;

  shared variable stats : check_stats_t;

  procedure report_result (ok : boolean; msg, detail : string) is
  begin
    stats.note(ok);
    if not ok then
      if stats.expecting then
        log_debug("expected failure: " & msg & " " & detail);
      else
        log_error("check failed: " & msg & " " & detail);
      end if;
    end if;
  end procedure;

  procedure check_true (cond : boolean; msg : string := "") is
  begin
    report_result(cond, msg, "condition false");
  end procedure;

  procedure check_equal (got, expected : integer; msg : string := "") is
  begin
    report_result(got = expected, msg, "got " & integer'image(got) & " expected " & integer'image(expected));
  end procedure;

  procedure check_equal (got, expected : string; msg : string := "") is
  begin
    report_result(got = expected, msg, "got '" & got & "' expected '" & expected & "'");
  end procedure;

  procedure check_equal (got, expected : std_logic_vector; msg : string := "") is
  begin
    report_result(got = expected, msg, "got " & to_hex(got) & " expected " & to_hex(expected));
  end procedure;

  procedure check_equal (got, expected : boolean; msg : string := "") is
  begin
    report_result(got = expected, msg, "got " & boolean'image(got) & " expected " & boolean'image(expected));
  end procedure;

  procedure check_near (got, expected, tol : integer; msg : string := "") is
  begin
    report_result(abs (got - expected) <= tol, msg, "got " & integer'image(got) & " near " & integer'image(expected));
  end procedure;

  procedure begin_expect_failures is
  begin
    stats.begin_expect;
  end procedure;

  impure function end_expect_failures return natural is
  begin
    return stats.end_expect;
  end function;

  impure function passed_count return natural is
  begin
    return stats.passed;
  end function;

  impure function failed_count return natural is
  begin
    return stats.failed;
  end function;
end package body fw_check;

use work.fw_types.all;
use work.fw_strings.all;

-- Test runner: select cases by a comma separated filter, count runs.
package fw_runner is
  type runner_t is protected
    procedure configure (spec : string);
    impure function selected (name : string) return boolean;
    procedure start (name : string);
    procedure finish_one;
    impure function runs return natural;
    impure function skipped return natural;
    impure function current return string;
  end protected;

end package fw_runner;

package body fw_runner is
  type runner_t is protected body
    variable filter : str_ptr_t := new string'("all");
    variable n_runs : natural := 0;
    variable n_skipped : natural := 0;
    variable cur : str_ptr_t := new string'("");

    procedure configure (spec : string) is
    begin
      deallocate(filter);
      filter := new string'(spec);
    end procedure;

    impure function selected (name : string) return boolean is
    begin
      if filter.all = "all" then
        return true;
      end if;
      for i in 0 to split_count(filter.all, ',') - 1 loop
        if split_nth(filter.all, ',', i) = name then
          return true;
        end if;
      end loop;
      return false;
    end function;

    procedure start (name : string) is
    begin
      deallocate(cur);
      cur := new string'(name);
      n_runs := n_runs + 1;
    end procedure;

    procedure finish_one is
    begin
      null;
    end procedure;

    impure function runs return natural is
    begin
      return n_runs;
    end function;

    impure function skipped return natural is
    begin
      return n_skipped;
    end function;

    impure function current return string is
    begin
      return cur.all;
    end function;
  end protected body;

end package body fw_runner;

use work.fw_runner.all;
use work.fw_logger.all;

-- The shared runner object and the test entry points.
package fw_run is
  shared variable the_runner : runner_t;
  impure function run_test (name : string) return boolean;
  procedure test_done;
end package fw_run;

package body fw_run is
  impure function run_test (name : string) return boolean is
  begin
    if the_runner.selected(name) then
      the_runner.start(name);
      log_info("start " & name);
      return true;
    end if;
    return false;
  end function;

  procedure test_done is
  begin
    log_info("done " & the_runner.current);
    the_runner.finish_one;
  end procedure;
end package body fw_run;

-- Context bundling the ieee packages the framework users need.
context fw_ctx is
  library ieee;
  use ieee.std_logic_1164.all;
  use ieee.numeric_std.all;
end context fw_ctx;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Adder with three architectures.
entity dut_adder is
  generic (width : positive := 8);
  port (
    a, b : in unsigned(width - 1 downto 0);
    cin : in std_logic;
    sum : out unsigned(width - 1 downto 0);
    cout : out std_logic
  );
end entity dut_adder;

architecture rtl of dut_adder is
begin
  process (all)
    variable t : unsigned(width downto 0);
    variable c : unsigned(0 downto 0);
  begin
    c(0) := cin;
    t := ('0' & a) + ('0' & b) + resize(c, width + 1);
    sum <= t(width - 1 downto 0);
    cout <= t(width);
  end process;
end architecture rtl;

architecture ripple of dut_adder is
  signal carry : std_logic_vector(width downto 0);
begin
  carry(0) <= cin;
  bits : for i in 0 to width - 1 generate
    sum(i) <= a(i) xor b(i) xor carry(i);
    carry(i + 1) <= (a(i) and b(i)) or (carry(i) and (a(i) xor b(i)));
  end generate bits;
  cout <= carry(width);
end architecture ripple;

architecture lookahead of dut_adder is
  signal g, p : std_logic_vector(width - 1 downto 0);
  signal c : std_logic_vector(width downto 0);
begin
  g <= std_logic_vector(a and b);
  p <= std_logic_vector(a xor b);
  wide : if width > 4 generate
    process (all)
      variable cv : std_logic_vector(width downto 0);
    begin
      cv(0) := cin;
      for i in 0 to width - 1 loop
        cv(i + 1) := g(i) or (p(i) and cv(i));
      end loop;
      c <= cv;
    end process;
  else generate
    c(0) <= cin;
    c(1) <= g(0) or (p(0) and c(0));
    rest : for i in 1 to width - 1 generate
      c(i + 1) <= g(i) or (p(i) and c(i));
    end generate rest;
  end generate;
  sum <= unsigned(p xor c(width - 1 downto 0));
  cout <= c(width);
end architecture lookahead;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Three adders side by side, bound by a configuration.
entity adder_bank is
  generic (width : positive := 8);
  port (
    a, b : in unsigned(width - 1 downto 0);
    cin : in std_logic;
    sum0, sum1, sum2 : out unsigned(width - 1 downto 0);
    co0, co1, co2 : out std_logic
  );
end entity adder_bank;

architecture a_all of adder_bank is
  component adder_c
    generic (width : positive := 8);
    port (
      a, b : in unsigned(width - 1 downto 0);
      cin : in std_logic;
      sum : out unsigned(width - 1 downto 0);
      cout : out std_logic
    );
  end component;
begin
  u0 : adder_c generic map (width) port map (a, b, cin, sum0, co0);
  u1 : adder_c generic map (width) port map (a, b, cin, sum1, co1);
  u2 : adder_c generic map (width) port map (a, b, cin, sum2, co2);
end architecture a_all;

configuration adder_bank_cfg of adder_bank is
  for a_all
    for u0 : adder_c
      use entity work.dut_adder(rtl);
    end for;
    for u1 : adder_c
      use entity work.dut_adder(ripple);
    end for;
    for u2 : adder_c
      use entity work.dut_adder(lookahead);
    end for;
  end for;
end configuration adder_bank_cfg;

library ieee;
use ieee.std_logic_1164.all;

-- Shift register with taps, built from generate statements.
entity shift_tap is
  generic (stages : positive := 4);
  port (
    clk : in std_logic;
    d : in std_logic;
    q : out std_logic_vector(stages - 1 downto 0)
  );
end entity shift_tap;

architecture rtl of shift_tap is
  signal r : std_logic_vector(stages - 1 downto 0) := (others => '0');
begin
  stage : for i in 0 to stages - 1 generate
    first : if i = 0 generate
      process (clk)
      begin
        if rising_edge(clk) then
          r(0) <= d;
        end if;
      end process;
    else generate
      process (clk)
      begin
        if rising_edge(clk) then
          r(i) <= r(i - 1);
        end if;
      end process;
    end generate first;
  end generate stage;
  q <= r;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
context work.fw_ctx;
use work.fw_types.all;
use work.fw_strings.all;
use work.fw_conv.all;
use work.fw_arr.all;
use work.fw_logger.all;
use work.fw_check.all;
use work.fw_run.all;

-- Testbench: four test cases run through the framework.
entity mini_framework_tb is
  generic (test_filter : string := "all");
end entity mini_framework_tb;

architecture tb of mini_framework_tb is
  package int_stk is new work.fw_gen_stack
    generic map (elem_t => log_level_t, default_elem => lvl_debug, img => level_tag);

  constant width : positive := 8;
  signal clk : std_logic := '0';
  signal finished : boolean := false;
  signal a, b : unsigned(width - 1 downto 0) := (others => '0');
  signal cin : std_logic := '0';
  signal s0, s1, s2 : unsigned(width - 1 downto 0);
  signal c0, c1, c2 : std_logic;
  signal d_in : std_logic := '0';
  signal taps : std_logic_vector(3 downto 0);
begin
  clk <= not clk after 5 ns when not finished else clk;

  bank : configuration work.adder_bank_cfg
    generic map (width => width)
    port map (a, b, cin, s0, s1, s2, c0, c1, c2);

  sr : entity work.shift_tap generic map (stages => 4) port map (clk, d_in, taps);

  main : process
    variable q : work.fw_queue.int_queue_t;
    variable ist : work.int_stack_pkg.stack_t;
    variable lst : int_stk.stack_t;
    variable v : int_vec_t(0 to 5);
    variable expect_sum : natural;
    variable rng : natural := 12345;
    variable pat : std_logic_vector(3 downto 0);

    procedure test_strings is
    begin
      for r in 1 to 12 loop
        check_equal(to_hex(r * 4099, 6), to_hex(to_slv(r * 4099, 24)), "hex agree");
        check_equal(parse_int(integer'image(-r * 37)), -r * 37, "parse round trip");
        check_equal(replace(repeat_str("ab,", r), ',', ';'), repeat_str("ab;", r), "replace loop");
        check_equal(split_count(repeat_str("x,", r), ','), r + 1, "split loop");
      end loop;
      check_equal(upper("MiXed 12"), "MIXED 12", "upper");
      check_equal(lower("MiXed 12"), "mixed 12", "lower");
      check_equal(strip("  " & HT & " hi there  "), "hi there", "strip");
      check_equal(starts_with("mini_framework", "mini"), true, "starts");
      check_equal(ends_with("mini_framework", "work"), true, "ends");
      check_equal(ends_with("ab", "xab"), false, "ends short");
      check_equal(find_char("a,b,c", ',', 3), 4, "find");
      check_equal(count_char("a,b,c", ','), 2, "count");
      check_equal(replace("a-b-c", '-', '_'), "a_b_c", "replace char");
      check_equal(replace("one two one", "one", "1"), "1 two 1", "replace str");
      check_equal(split_count("x, y ,z", ','), 3, "split count");
      check_equal(split_nth("x, y ,z", ',', 1), "y", "split nth");
      check_equal(parse_int(" -42 "), -42, "parse");
      check_equal(pad_left("7", 3, '0'), "007", "pad");
      check_equal(repeat_str("ab", 3), "ababab", "repeat");
      check_equal(to_hex(255, 4), "00FF", "hex nat");
      check_equal(to_hex(std_logic_vector'(x"3a5")), "3A5", "hex slv");
      check_equal(to_hex(std_logic_vector'("10X1")), "X", "hex unknown");
      check_equal(level_tag(lvl_warning), "WARNING", "level tag");
      check_equal(to_string(iota(4, 3)), "[3,4,5,6]", "int vec string");
      check_equal(bin_str(to_slv(5, 6)), "000101", "bin str");
      check_equal(slv_from_hex("1f"), to_slv(31, 8), "from hex");
      check_equal(popcount(x"f0f1"), 9, "popcount");
      check_equal(reverse_bits(std_logic_vector'("1100")), std_logic_vector'("0011"), "reverse");
      check_equal(rotate_left_slv(x"81", 1), x"03", "rotate");
      check_true(parity(x"07") = '1', "parity");
    end procedure;

    procedure test_containers is
      variable popped : integer;
      variable arr : slv_vec_t(0 to 2)(7 downto 0);
    begin
      for i in 1 to 20000 loop
        q.push(i);
      end loop;
      for i in 1 to 20000 loop
        popped := q.pop;
        check_equal(popped, i, "bulk order");
      end loop;
      for i in 1 to 40 loop
        q.push(i * i);
      end loop;
      check_equal(q.size, 40, "queue size");
      check_true(q.capacity >= 40, "queue grew");
      for i in 1 to 25 loop
        popped := q.pop;
        check_equal(popped, i * i, "queue order");
      end loop;
      for i in 41 to 70 loop
        q.push(i);
      end loop;
      check_equal(q.size, 45, "queue size 2");
      check_equal(q.peek, 26 * 26, "peek");
      q.clear;
      check_true(q.empty, "cleared");
      for i in 0 to 20 loop
        ist.push(i * 3);
      end loop;
      check_equal(ist.depth, 21, "stack depth");
      check_equal(ist.pop, 60, "stack pop");
      check_equal(ist.top, 57, "stack top");
      lst.push(lvl_error);
      lst.push(lvl_info);
      check_equal(lst.dump, "ERROR INFO", "stack dump");
      check_true(lst.pop = lvl_info, "enum pop");
      popped := lst.depth;
      check_equal(popped, 1, "enum depth");
      v := (5, 3, 9, -2, 7, 1);
      check_equal(to_string(sorted(v)), "[-2,1,3,5,7,9]", "sorted");
      check_equal(to_string(reverse(v)), "[1,7,-2,9,3,5]", "reverse");
      check_equal(sum_of(concat(v, iota(3, 10))), 23 + 33, "concat sum");
      check_equal(max_of(scale(v, -2)), 4, "scale max");
      check_equal(min_of(v), -2, "min");
      arr := to_slv_vec((1, 2, 3), 8);
      check_equal(arr(2), x"03", "slv vec");
    end procedure;

    procedure test_adders is
    begin
      for i in 0 to 16000 loop
        rng := (rng * 1103 + 12345) mod 65536;
        a <= to_unsigned(rng mod 256, width);
        b <= to_unsigned((rng / 256) mod 256, width);
        expect_sum := (rng mod 256) + ((rng / 256) mod 256);
        cin <= '0';
        if i mod 3 = 0 then
          cin <= '1';
          expect_sum := expect_sum + 1;
        end if;
        wait until rising_edge(clk);
        wait for 1 ns;
        check_equal(to_integer(s0), expect_sum mod 256, "rtl sum");
        check_equal(to_integer(s1), expect_sum mod 256, "ripple sum");
        check_equal(to_integer(s2), expect_sum mod 256, "lookahead sum");
        check_equal(c0 = '1', expect_sum > 255, "rtl carry");
        check_equal(c1 = '1', expect_sum > 255, "ripple carry");
        check_equal(c2 = '1', expect_sum > 255, "lookahead carry");
      end loop;
      for i in 0 to 15 loop
        d_in <= '0';
        if i mod 5 < 2 then
          d_in <= '1';
        end if;
        wait until rising_edge(clk);
        wait for 1 ns;
      end loop;
      pat := taps;
      check_true(not is_x(pat), "taps known");
    end procedure;

    procedure test_log_and_check is
      variable seen : natural;
    begin
      the_log.reset;
      begin_expect_failures;
      check_equal(1, 2, "deliberate");
      check_equal(string'("ab"), string'("ac"), "deliberate string");
      check_true(false, "deliberate true");
      seen := end_expect_failures;
      check_equal(seen, 3, "expected failures seen");
      check_equal(failed_count, 0, "no real failures yet");
      log_warning("first warning");
      log_error("an error");
      log_debug("hidden");
      check_equal(the_log.count(lvl_warning), 1, "warning count");
      check_equal(the_log.count(lvl_debug), 4, "debug count");
      check_equal(the_log.mem_count, 2, "memory lines");
      check_equal(the_log.mem_line(1), "ERROR: an error", "memory text");
      check_true(the_log.handler(lvl_info) = h_console, "info handler");
      check_true(the_log.threshold = lvl_info, "threshold");
      for i in 1 to 40 loop
        log_warning("w" & integer'image(i));
      end loop;
      check_equal(the_log.mem_count, 42, "wrapped count");
    end procedure;
  begin
    the_runner.configure(test_filter);
    if run_test("test_strings") then
      test_strings;
      test_done;
    end if;
    if run_test("test_containers") then
      test_containers;
      test_done;
    end if;
    if run_test("test_adders") then
      test_adders;
      test_done;
    end if;
    if run_test("test_log_and_check") then
      test_log_and_check;
      test_done;
    end if;
    assert the_runner.runs = 4 report "unexpected number of tests run" severity failure;
    assert failed_count = 0 report "checks failed" severity failure;
    log_info("checks passed: " & integer'image(passed_count));
    report "PASS" severity note;
    finished <= true;
    wait;
  end process;
end architecture tb;
