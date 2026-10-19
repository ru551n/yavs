-- (from the differential corpus)
-- ref=hand: the reference simulator rejects these attributes in any subprogram body; the
-- second and third reference simulators give this output (they print no delta or path).
-- 'stable, 'quiet, 'delayed and 'transaction of signals that are not formal parameters, named in
-- a procedure declared in a process, a function declared in the architecture and a procedure
-- of a package body naming a package signal (LRM
-- 4.2.2.2 forbids them only of formal signal parameters; 16.2.4: the implicit signal belongs
-- to the region enclosing the subprogram) (#240).
package implicit_pkg is
  signal pk : integer := 0;
  procedure show_pkg(tag : string);
end package;

package body implicit_pkg is
  procedure show_pkg(tag : string) is
  begin
    report tag & " pk'stable " & boolean'image(pk'stable)
      & " pk'quiet(1 ns) " & boolean'image(pk'quiet(1 ns))
      & " pk'delayed(1 ns) " & integer'image(pk'delayed(1 ns));
  end procedure;
end package body;

use work.implicit_pkg.all;

entity implicit_in_subprogram is
end entity;

architecture test of implicit_in_subprogram is
  type cell is record
    ok : boolean;
    n  : natural;
  end record;
  signal i : integer := 0;
  signal r : cell := (true, 1);

  impure function arch_view return string is
  begin
    return "i'stable(2 ns) " & boolean'image(i'stable(2 ns))
      & " i'quiet " & boolean'image(i'quiet)
      & " i'delayed(1 ns) " & integer'image(i'delayed(1 ns))
      & " r'stable " & boolean'image(r'stable);
  end function;
begin
  drv : process
  begin
    wait for 1 ns;
    i <= 1;
    wait for 3 ns;
    i <= 1;
    r.n <= 2;
    pk <= 3;
    wait for 3 ns;
    r.ok <= true;
    wait;
  end process;

  mon : process
    variable n : natural := 0;
    procedure show(tag : string) is
      variable d : cell;
    begin
      d := r'delayed(2 ns);
      report tag & " i'stable " & boolean'image(i'stable)
        & " i'quiet(1 ns) " & boolean'image(i'quiet(1 ns))
        & " i'delayed(2 ns) " & integer'image(i'delayed(2 ns))
        & " r'quiet(1 ns) " & boolean'image(r'quiet(1 ns))
        & " r'delayed(2 ns).n " & integer'image(d.n)
        & " / " & arch_view;
    end procedure;
    procedure wait_transaction is
    begin
      wait on i'transaction, r'transaction;
      n := n + 1;
    end procedure;
  begin
    for k in 1 to 3 loop
      wait_transaction;
      show("transaction " & integer'image(n));
      wait for 1 ns;
      show("1 ns later");
      show_pkg("package");
    end loop;
    wait;
  end process;
end architecture;
