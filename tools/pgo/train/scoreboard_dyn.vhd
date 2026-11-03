-- pgo: top=scoreboard_dyn_tb std=2008 stop=2ms analyze=3
-- Reproduces a verification-style testbench built on protected types: a scoreboard on a linked
-- list, a random generator, coverage bins, an integer list and a growing string buffer, used from
-- two interacting clocked processes and from one long stress process.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Transaction record and helpers.
package sb_types is
  type kind_t is (k_read, k_write, k_atomic, k_flush);
  type txn_t is record
    id : natural;
    kind : kind_t;
    addr : std_logic_vector(11 downto 0);
    data : std_logic_vector(15 downto 0);
    valid : std_logic;
  end record;
  type txn_vec_t is array (natural range <>) of txn_t;
  constant null_txn : txn_t := (0, k_read, (others => '0'), (others => '0'), '0');
  function make_txn (id : natural; kind : kind_t; addr : natural; data : natural) return txn_t;
  function expected_of (t : txn_t) return txn_t;
  function same (a, b : txn_t) return boolean;
  function img (t : txn_t) return string;
end package sb_types;

package body sb_types is
  function make_txn (id : natural; kind : kind_t; addr : natural; data : natural) return txn_t is
    variable t : txn_t;
  begin
    t.id := id;
    t.kind := kind;
    t.addr := std_logic_vector(to_unsigned(addr mod 4096, 12));
    t.data := std_logic_vector(to_unsigned(data mod 65536, 16));
    t.valid := '1';
    return t;
  end function;

  -- What the device under test does to a transaction.
  function expected_of (t : txn_t) return txn_t is
    variable r : txn_t := t;
  begin
    case t.kind is
      when k_read => r.data := t.data xor (t.addr & "0000");
      when k_write => r.data := std_logic_vector(unsigned(t.data) + unsigned(t.addr));
      when k_atomic => r.data := not t.data;
      when k_flush => r.data := (others => '0');
    end case;
    return r;
  end function;

  function same (a, b : txn_t) return boolean is
  begin
    return a.id = b.id and a.kind = b.kind and a.addr = b.addr and a.data = b.data;
  end function;

  function img (t : txn_t) return string is
  begin
    return "#" & integer'image(t.id) & " " & kind_t'image(t.kind) & " a=" &
           to_hstring(t.addr) & " d=" & to_hstring(t.data);
  end function;
end package body sb_types;

use work.sb_types.all;

-- The protected types.
package sb_prot is
  type rand_t is protected
    procedure seed (s : positive);
    impure function next_val return natural;
    impure function uniform (lo, hi : integer) return integer;
    impure function chance (percent : natural) return boolean;
  end protected;

  type cov_t is protected
    procedure define_bin (index : natural; lo, hi : integer);
    procedure sample (value : integer);
    impure function hits (index : natural) return natural;
    impure function total return natural;
    impure function uncovered return natural;
  end protected;

  type buf_t is protected
    procedure append (s : string);
    procedure append_int (v : integer);
    procedure clear;
    impure function length return natural;
    impure function capacity return natural;
    impure function text return string;
  end protected;

  type list_t is protected
    procedure push_back (v : integer);
    procedure push_front (v : integer);
    impure function pop_front return integer;
    impure function remove_value (v : integer) return boolean;
    impure function contains (v : integer) return boolean;
    impure function size return natural;
    impure function sum return integer;
    procedure clear;
  end protected;

  type scoreboard_t is protected
    procedure insert (t : txn_t);
    impure function check (actual : txn_t) return boolean;
    procedure check_any (actual : txn_t; ok : out boolean);
    procedure flush;
    procedure reset_counts;
    impure function pending return natural;
    impure function matched return natural;
    impure function errors return natural;
  end protected;
end package sb_prot;

package body sb_prot is
  -- Park-Miller generator, no library randomisation.
  type rand_t is protected body
    variable state : positive := 1;

    procedure seed (s : positive) is
    begin
      state := s;
    end procedure;

    impure function next_val return natural is
      constant a : natural := 16807;
      constant q : natural := 127773;
      constant r : natural := 2836;
      variable hi, lo : natural;
      variable t : integer;
    begin
      hi := state / q;
      lo := state mod q;
      t := a * lo - r * hi;
      if t <= 0 then
        t := t + 2147483647;
      end if;
      state := t;
      return t;
    end function;

    impure function uniform (lo, hi : integer) return integer is
    begin
      return lo + next_val mod (hi - lo + 1);
    end function;

    impure function chance (percent : natural) return boolean is
    begin
      return next_val mod 100 < percent;
    end function;
  end protected body;

  type cov_t is protected body
    type int_arr_t is array (0 to 15) of integer;
    type nat_arr_t is array (0 to 15) of natural;
    variable lows, highs : int_arr_t := (others => 0);
    variable counts : nat_arr_t := (others => 0);
    variable defined : natural := 0;
    variable samples : natural := 0;

    procedure define_bin (index : natural; lo, hi : integer) is
    begin
      lows(index) := lo;
      highs(index) := hi;
      defined := maximum(defined, index + 1);
    end procedure;

    procedure sample (value : integer) is
    begin
      samples := samples + 1;
      for i in 0 to defined - 1 loop
        if value >= lows(i) and value <= highs(i) then
          counts(i) := counts(i) + 1;
        end if;
      end loop;
    end procedure;

    impure function hits (index : natural) return natural is
    begin
      return counts(index);
    end function;

    impure function total return natural is
    begin
      return samples;
    end function;

    impure function uncovered return natural is
      variable n : natural := 0;
    begin
      for i in 0 to defined - 1 loop
        if counts(i) = 0 then
          n := n + 1;
        end if;
      end loop;
      return n;
    end function;
  end protected body;

  type buf_t is protected body
    type str_ptr_t is access string;
    variable data : str_ptr_t := new string(1 to 16);
    variable len : natural := 0;

    procedure reserve (extra : natural) is
      variable bigger : str_ptr_t;
      variable cap : natural := data'length;
    begin
      if len + extra > cap then
        while len + extra > cap loop
          cap := cap * 2;
        end loop;
        bigger := new string(1 to cap);
        bigger(1 to len) := data(1 to len);
        deallocate(data);
        data := bigger;
      end if;
    end procedure;

    procedure append (s : string) is
    begin
      reserve(s'length);
      data(len + 1 to len + s'length) := s;
      len := len + s'length;
    end procedure;

    procedure append_int (v : integer) is
    begin
      append(integer'image(v));
    end procedure;

    procedure clear is
    begin
      len := 0;
    end procedure;

    impure function length return natural is
    begin
      return len;
    end function;

    impure function capacity return natural is
    begin
      return data'length;
    end function;

    impure function text return string is
    begin
      return data(1 to len);
    end function;
  end protected body;

  type list_t is protected body
    type node_t;
    type node_ptr_t is access node_t;
    type node_t is record
      item : integer;
      next_n : node_ptr_t;
    end record;
    variable head, tail : node_ptr_t := null;
    variable count : natural := 0;

    procedure push_back (v : integer) is
      variable n : node_ptr_t := new node_t'(item => v, next_n => null);
    begin
      if tail = null then
        head := n;
      else
        tail.next_n := n;
      end if;
      tail := n;
      count := count + 1;
    end procedure;

    procedure push_front (v : integer) is
      variable n : node_ptr_t := new node_t'(item => v, next_n => head);
    begin
      head := n;
      if tail = null then
        tail := n;
      end if;
      count := count + 1;
    end procedure;

    impure function pop_front return integer is
      variable n : node_ptr_t := head;
      variable v : integer;
    begin
      assert n /= null report "pop_front on empty list" severity failure;
      v := n.item;
      head := n.next_n;
      if head = null then
        tail := null;
      end if;
      deallocate(n);
      count := count - 1;
      return v;
    end function;

    impure function remove_value (v : integer) return boolean is
      variable prev, cur : node_ptr_t := null;
    begin
      cur := head;
      while cur /= null loop
        if cur.item = v then
          if prev = null then
            head := cur.next_n;
          else
            prev.next_n := cur.next_n;
          end if;
          if cur = tail then
            tail := prev;
          end if;
          deallocate(cur);
          count := count - 1;
          return true;
        end if;
        prev := cur;
        cur := cur.next_n;
      end loop;
      return false;
    end function;

    impure function contains (v : integer) return boolean is
      variable cur : node_ptr_t := head;
    begin
      while cur /= null loop
        if cur.item = v then
          return true;
        end if;
        cur := cur.next_n;
      end loop;
      return false;
    end function;

    impure function size return natural is
    begin
      return count;
    end function;

    impure function sum return integer is
      variable cur : node_ptr_t := head;
      variable s : integer := 0;
    begin
      while cur /= null loop
        s := s + cur.item;
        cur := cur.next_n;
      end loop;
      return s;
    end function;

    procedure clear is
      variable n : node_ptr_t;
    begin
      while head /= null loop
        n := head;
        head := head.next_n;
        deallocate(n);
      end loop;
      tail := null;
      count := 0;
    end procedure;
  end protected body;

  type scoreboard_t is protected body
    type node_t;
    type node_ptr_t is access node_t;
    type node_t is record
      item : txn_t;
      next_n : node_ptr_t;
    end record;
    variable head, tail : node_ptr_t := null;
    variable n_pending : natural := 0;
    variable n_matched : natural := 0;
    variable n_errors : natural := 0;

    procedure insert (t : txn_t) is
      variable n : node_ptr_t := new node_t'(item => t, next_n => null);
    begin
      if tail = null then
        head := n;
      else
        tail.next_n := n;
      end if;
      tail := n;
      n_pending := n_pending + 1;
    end procedure;

    impure function check (actual : txn_t) return boolean is
      variable n : node_ptr_t := head;
      variable ok : boolean;
    begin
      if n = null then
        n_errors := n_errors + 1;
        return false;
      end if;
      ok := same(n.item, actual);
      head := n.next_n;
      if head = null then
        tail := null;
      end if;
      deallocate(n);
      n_pending := n_pending - 1;
      if ok then
        n_matched := n_matched + 1;
      else
        n_errors := n_errors + 1;
      end if;
      return ok;
    end function;

    procedure check_any (actual : txn_t; ok : out boolean) is
      variable prev, cur : node_ptr_t := null;
    begin
      cur := head;
      while cur /= null loop
        if same(cur.item, actual) then
          if prev = null then
            head := cur.next_n;
          else
            prev.next_n := cur.next_n;
          end if;
          if cur = tail then
            tail := prev;
          end if;
          deallocate(cur);
          n_pending := n_pending - 1;
          n_matched := n_matched + 1;
          ok := true;
          return;
        end if;
        prev := cur;
        cur := cur.next_n;
      end loop;
      n_errors := n_errors + 1;
      ok := false;
    end procedure;

    procedure flush is
      variable n : node_ptr_t;
    begin
      while head /= null loop
        n := head;
        head := head.next_n;
        deallocate(n);
      end loop;
      tail := null;
      n_pending := 0;
    end procedure;

    procedure reset_counts is
    begin
      n_matched := 0;
      n_errors := 0;
    end procedure;

    impure function pending return natural is
    begin
      return n_pending;
    end function;

    impure function matched return natural is
    begin
      return n_matched;
    end function;

    impure function errors return natural is
    begin
      return n_errors;
    end function;
  end protected body;
end package body sb_prot;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.sb_types.all;
use work.sb_prot.all;

entity scoreboard_dyn_tb is
end entity scoreboard_dyn_tb;

architecture tb of scoreboard_dyn_tb is
  constant n_txn : natural := 50000;
  signal clk : std_logic := '0';
  signal finished : boolean := false;
  signal in_txn, out_txn : txn_t := null_txn;
  signal pipe : txn_vec_t(0 to 1) := (others => null_txn);
  signal producer_done : boolean := false;
  signal monitor_done : boolean := false;
  signal sent_count : natural := 0;

  shared variable sb : scoreboard_t;
  shared variable rng : rand_t;
  shared variable cov : cov_t;
  shared variable msgs : buf_t;
begin
  clk <= not clk after 5 ns when not finished else clk;

  -- Device under test: three register stages.
  dut : process (clk)
  begin
    if rising_edge(clk) then
      pipe(0) <= in_txn;
      pipe(1) <= pipe(0);
      if pipe(1).valid = '1' then
        out_txn <= expected_of(pipe(1));
      else
        out_txn <= null_txn;
      end if;
    end if;
  end process;

  -- Producer: random transactions, recorded in the scoreboard as they are sent.
  producer : process
    variable t : txn_t;
    variable n : natural := 0;
    variable k : kind_t;
  begin
    rng.seed(s => 20240601);
    cov.define_bin(index => 0, lo => 0, hi => 1023);
    cov.define_bin(index => 1, lo => 1024, hi => 2047);
    cov.define_bin(index => 2, lo => 2048, hi => 3071);
    cov.define_bin(index => 3, lo => 3072, hi => 4095);
    cov.define_bin(index => 4, lo => 0, hi => 4095);
    wait until rising_edge(clk);
    while n < n_txn loop
      if rng.chance(percent => 75) then
        k := kind_t'val(rng.uniform(lo => 0, hi => 3));
        t := make_txn(id => n, kind => k, addr => rng.uniform(lo => 0, hi => 4095),
                      data => rng.uniform(lo => 0, hi => 65535));
        sb.insert(t => expected_of(t));
        cov.sample(value => to_integer(unsigned(t.addr)));
        in_txn <= t;
        n := n + 1;
      else
        in_txn <= null_txn;
      end if;
      wait until rising_edge(clk);
    end loop;
    in_txn <= null_txn;
    sent_count <= n;
    producer_done <= true;
    wait;
  end process;

  -- Monitor: checks every valid output against the scoreboard, in order.
  monitor : process
    variable seen : natural := 0;
    variable ok : boolean;
  begin
    loop
      wait until rising_edge(clk);
      if out_txn.valid = '1' then
        ok := sb.check(actual => out_txn);
        assert ok report "mismatch: " & img(out_txn) severity failure;
        seen := seen + 1;
      end if;
      exit when producer_done and seen = sent_count;
    end loop;
    monitor_done <= true;
    wait;
  end process;

  main : process
    variable ok : boolean;
    variable v : integer;
    variable t, u : txn_t;
    variable expected_sum : integer;
    variable ids : txn_vec_t(0 to 63);
    variable ooo : scoreboard_t;
    variable lst : list_t;
    variable cov2 : cov_t;
    variable local_msgs : buf_t;
    variable total_matched : natural;
  begin
    wait until monitor_done;
    assert sb.pending = 0 report "scoreboard not drained" severity failure;
    assert sb.matched = n_txn report "wrong match count" severity failure;
    total_matched := sb.matched;
    assert sb.errors = 0 report "scoreboard errors" severity failure;
    assert cov.total = n_txn report "coverage sample count" severity failure;
    assert cov.hits(index => 4) = n_txn report "whole range bin" severity failure;
    assert cov.uncovered = 0 report "coverage hole" severity failure;

    -- Error paths: a wrong item, an item with nothing expected, flush.
    sb.reset_counts;
    t := make_txn(id => 1, kind => k_write, addr => 16#10#, data => 5);
    sb.insert(t => t);
    u := t;
    u.data := x"ffff";
    ok := sb.check(actual => u);
    assert not ok report "mismatch not detected" severity failure;
    ok := sb.check(actual => t);
    assert not ok report "empty check not detected" severity failure;
    assert sb.errors = 2 report "error count" severity failure;
    sb.insert(t => t);
    sb.insert(t => u);
    sb.flush;
    assert sb.pending = 0 report "flush left items" severity failure;
    sb.reset_counts;

    -- Out of order matching with a second scoreboard.
    for round in 0 to 119 loop
      for i in ids'range loop
        ids(i) := make_txn(id => round * 64 + i, kind => kind_t'val(i mod 4),
                           addr => rng.uniform(lo => 0, hi => 4095), data => i * 977);
        ooo.insert(t => ids(i));
      end loop;
      for i in ids'range loop
        t := ids((i * 37 + round) mod 64);
        ooo.check_any(actual => t, ok => ok);
        if i = 0 then
          assert ok report "ooo first" severity failure;
        end if;
        if not ok then
          -- a transaction checked twice is an expected error here
          assert ooo.errors > 0 report "ooo error not counted" severity failure;
        end if;
      end loop;
      ooo.flush;
    end loop;
    assert ooo.matched + ooo.errors = 120 * 64 report "ooo totals" severity failure;
    assert ooo.matched >= 120 * 40 report "ooo matched too few" severity failure;

    -- Integer list: mixed operations, tracked against an array model.
    expected_sum := 0;
    for i in 1 to 12000 loop
      v := rng.uniform(lo => 1, hi => 500);
      case i mod 5 is
        when 0 =>
          lst.push_front(v => v);
          expected_sum := expected_sum + v;
        when 1 | 2 =>
          lst.push_back(v => v);
          expected_sum := expected_sum + v;
        when 3 =>
          if lst.size > 0 then
            expected_sum := expected_sum - lst.pop_front;
          end if;
        when others =>
          if lst.remove_value(v => v) then
            expected_sum := expected_sum - v;
          end if;
      end case;
      if i mod 500 = 0 then
        assert lst.sum = expected_sum report "list sum drift" severity failure;
      end if;
    end loop;
    assert lst.sum = expected_sum report "list sum" severity failure;
    lst.clear;
    assert lst.size = 0 report "list clear" severity failure;
    lst.push_back(v => 7);
    assert lst.contains(v => 7) and not lst.contains(v => 8) report "list contains" severity failure;

    -- Second coverage object and the string buffer.
    for i in 0 to 7 loop
      cov2.define_bin(index => i, lo => i * 10, hi => i * 10 + 9);
    end loop;
    for i in 1 to 4000 loop
      cov2.sample(value => rng.uniform(lo => 0, hi => 79));
    end loop;
    assert cov2.uncovered = 0 report "cov2 hole" severity failure;
    for i in 1 to 600 loop
      local_msgs.append(s => "item ");
      local_msgs.append_int(v => i);
      local_msgs.append(s => ";");
      if i mod 100 = 0 then
        msgs.append(s => local_msgs.text(1 to 12));
        local_msgs.clear;
      end if;
    end loop;
    assert local_msgs.length = 0 report "buffer not cleared" severity failure;
    assert local_msgs.capacity >= 16 report "buffer capacity" severity failure;
    assert msgs.length = 72 report "summary length " & integer'image(msgs.length) severity failure;
    assert msgs.text(1 to 12) = "item 1;item " report "summary text" severity failure;

    report "scoreboard matched " & integer'image(total_matched) & " items, summary: " &
           msgs.text(1 to 12) severity note;
    report "PASS" severity note;
    finished <= true;
    wait;
  end process;
end architecture tb;
