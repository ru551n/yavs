-- difftest: top=elab_calls std=2008
-- Impure functions called during elaboration (#77, LRM 14.4): generic map actuals and
-- constant initial values call functions that create entries in package state (a shared
-- variable of a protected type). The calls run in declaration order, after the packages
-- are elaborated, and their side effects are what the processes see at run time. A package
-- constant with an impure initial value is evaluated once, when its package is elaborated,
-- also when elaboration reads it.
package elab_calls_reg is
  type handle_t is record
    id  : natural;
    tag : character;
  end record;
  type registry_t is protected
    impure function add(tag : character) return natural;
    impure function count return natural;
    impure function tag_of(id : natural) return character;
  end protected;
  impure function new_handle(tag : character) return handle_t;
  impure function handles return natural;
  impure function tag_of(h : handle_t) return character;
  function plain(n : natural) return natural;
end package;

package body elab_calls_reg is
  type registry_t is protected body
    type tags_t is array (0 to 15) of character;
    variable tags : tags_t := (others => ' ');
    variable n : natural := 0;
    impure function add(tag : character) return natural is
    begin
      tags(n) := tag;
      n := n + 1;
      return n - 1;
    end function;
    impure function count return natural is
    begin
      return n;
    end function;
    impure function tag_of(id : natural) return character is
    begin
      return tags(id);
    end function;
  end protected body;

  shared variable reg : registry_t;

  impure function new_handle(tag : character) return handle_t is
  begin
    return (id => reg.add(tag), tag => tag);
  end function;
  impure function handles return natural is
  begin
    return reg.count;
  end function;
  impure function tag_of(h : handle_t) return character is
  begin
    return reg.tag_of(h.id);
  end function;
  function plain(n : natural) return natural is
  begin
    return n * 2;
  end function;
end package body;

use work.elab_calls_reg.all;
package elab_calls_cfg is
  constant root : handle_t := new_handle('r');
  impure function after_root(h : handle_t := root) return natural;
  impure function at_start return natural;
end package;

package body elab_calls_cfg is
  impure function after_root(h : handle_t := root) return natural is
  begin
    return handles - h.id;
  end function;
  -- NOW is 0 fs during elaboration.
  impure function at_start return natural is
  begin
    return now / 1 fs;
  end function;
end package body;

use work.elab_calls_reg.all;
entity elab_calls_child is
  generic (h : handle_t; width : natural);
end entity;
architecture a of elab_calls_child is
  constant own : handle_t := new_handle('c');
  signal v : bit_vector(width - 1 downto 0);
begin
  process
  begin
    report "child " & h.tag & " id " & integer'image(h.id) & " own " & integer'image(own.id)
      & " width " & integer'image(v'length);
    assert tag_of(h) = h.tag severity failure;
    assert tag_of(own) = 'c' severity failure;
    wait;
  end process;
end architecture;

use work.elab_calls_reg.all;
use work.elab_calls_cfg.all;
entity elab_calls is
end entity;
architecture a of elab_calls is
  constant first : handle_t := new_handle('a');
  -- The number of handles so far sizes a signal: the calls have run in order.
  signal sized : bit_vector(1 to handles);
  signal from_root : bit_vector(1 to after_root);
  constant name_v : string := "n" & to_string(handles) & to_string('x') & to_string(true);
  signal t : bit_vector(0 to at_start);
begin
  u1 : entity work.elab_calls_child generic map (h => new_handle('x'), width => plain(handles));
  u2 : entity work.elab_calls_child generic map (h => new_handle('y'), width => handles);
  process
  begin
    report "top first " & integer'image(first.id) & " sized " & integer'image(sized'length)
      & " root " & integer'image(root.id) & " " & root.tag & " from_root "
      & integer'image(from_root'length) & " " & name_v & " t " & integer'image(t'length);
    wait for 1 ns;
    report "handles " & integer'image(handles);
    assert handles = 6 severity failure;
    wait;
  end process;
end architecture;
