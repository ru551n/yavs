-- difftest: top=access_vars relaxed
-- A coverage-style package (accepted with the relaxed switch): shared variables of
-- non-protected types holding access values, a linked list of bins with line names, a binary
-- tree and a log record whose line is replaced thousands of times without deallocate, so the
-- shared heap fills with garbage and is collected while the list, the tree, a region's
-- shared line and record and a process's own list (reachable only from its variables)
-- must survive.
-- Bins are added, hit and deallocated by different processes. Every process acts at a time
-- of its own, so the output does not depend on process order.
use std.textio.all;
package access_vars_p is
  type node;
  type node_ptr is access node;
  type node is record
    id   : integer;
    hits : integer;
    name : line;
    link : node_ptr;
  end record;
  type tree;
  type tree_ptr is access tree;
  type tree is record
    key         : integer;
    left, right : tree_ptr;
  end record;
  type log_t is record
    count : integer;
    last  : line;
  end record;
  shared variable bins : node_ptr;
  shared variable root : tree_ptr;
  shared variable log : log_t := (0, null);
  procedure add_bin(id : integer; name : string);
  procedure hit(id : integer);
  procedure remove_bin(id : integer);
  procedure insert(key : integer);
  impure function tree_sum return integer;
  impure function bins_image return string;
  procedure note(s : string);
end package;

package body access_vars_p is
  procedure add_bin(id : integer; name : string) is
  begin
    bins := new node'(id, 0, new string'(name), bins);
  end procedure;

  procedure hit(id : integer) is
    variable p : node_ptr := bins;
  begin
    while p /= null loop
      if p.id = id then
        p.hits := p.hits + 1;
      end if;
      p := p.link;
    end loop;
  end procedure;

  procedure remove_bin(id : integer) is
    variable p    : node_ptr := bins;
    variable prev : node_ptr;
  begin
    while p /= null loop
      if p.id = id then
        if prev = null then
          bins := p.link;
        else
          prev.link := p.link;
        end if;
        deallocate(p.name);
        deallocate(p);
        return;
      end if;
      prev := p;
      p := p.link;
    end loop;
  end procedure;

  procedure insert(key : integer) is
    variable t : tree_ptr;
  begin
    if root = null then
      root := new tree'(key, null, null);
      return;
    end if;
    t := root;
    loop
      if key < t.key then
        if t.left = null then
          t.left := new tree'(key, null, null);
          return;
        end if;
        t := t.left;
      else
        if t.right = null then
          t.right := new tree'(key, null, null);
          return;
        end if;
        t := t.right;
      end if;
    end loop;
  end procedure;

  procedure sum(variable t : in tree_ptr; variable acc : inout integer) is
  begin
    if t /= null then
      acc := acc + t.key;
      sum(t.left, acc);
      sum(t.right, acc);
    end if;
  end procedure;

  impure function tree_sum return integer is
    variable acc : integer := 0;
  begin
    sum(root, acc);
    return acc;
  end function;

  impure function bins_image return string is
    variable l : line;
    variable p : node_ptr := bins;
  begin
    while p /= null loop
      write(l, p.name.all & "=" & integer'image(p.hits) & " ");
      p := p.link;
    end loop;
    if l = null then
      return "(none)";
    end if;
    return l.all;
  end function;

  procedure note(s : string) is
  begin
    log.count := log.count + 1;
    log.last := new string'(s);
  end procedure;
end package body;

use std.textio.all;
use work.access_vars_p.all;
entity access_vars is
end entity;

architecture a of access_vars is
  shared variable last : line;
  shared variable recent : log_t := (0, null);
begin
  -- Adds bins at 10 ns * (k - 1) and logs heavily (garbage) for the next 4 ns.
  adder : process
  begin
    for k in 1 to 8 loop
      add_bin(k, "bin" & integer'image(k));
      for i in 1 to 40 loop
        for j in 1 to 100 loop
          note("adder step " & integer'image(k) & " burst " & integer'image(i * 100 + j));
        end loop;
        wait for 100 ps;
      end loop;
      wait for 6 ns;
    end loop;
    wait;
  end process;

  -- Hits bins at 10 ns * k - 5 ns and keeps a list of its own across the waits.
  hitter : process
    variable mine : node_ptr;
    variable n    : integer := 0;
    variable p    : node_ptr;
  begin
    wait for 5 ns;
    for k in 1 to 8 loop
      hit(k);
      hit((k + 1) / 2);
      mine := new node'(k, k * k, new string'("mine" & integer'image(k)), mine);
      last := new string'("hitter " & integer'image(k));
      recent := (k, new string'("recent " & integer'image(k)));
      wait for 10 ns;
    end loop;
    p := mine;
    while p /= null loop
      n := n + p.hits;
      p := p.link;
    end loop;
    report "own list: " & mine.name.all & " sum=" & integer'image(n);
    wait;
  end process;

  -- Builds a tree at 10 ns * k - 3 ns and removes bins late.
  treeer : process
  begin
    wait for 7 ns;
    for k in 1 to 8 loop
      insert((k * 37) mod 17);
      insert((k * 53) mod 23);
      wait for 10 ns;
    end loop;
    remove_bin(3);
    remove_bin(8);
    remove_bin(1);
    report "removed 3 8 1";
    wait;
  end process;

  -- Reports at 10 ns * k - 1 ns.
  checker : process
  begin
    wait for 9 ns;
    for k in 1 to 9 loop
      report "t=" & integer'image(k) & " bins: " & bins_image & " tree=" &
        integer'image(tree_sum) & " log=" & integer'image(log.count) & " " & log.last.all &
        " last=" & last.all & " " & recent.last.all;
      wait for 10 ns;
    end loop;
    assert log.count = 32000 and tree_sum = 148 and bins.name.all = "bin7" severity failure;
    wait;
  end process;
end architecture;
