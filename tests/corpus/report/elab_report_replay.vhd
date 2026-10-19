-- difftest: top=elab_report_replay ref=hand
-- ref=hand: as elab_report, the harness runs `elab` and `run`, each elaborating the design, so
-- each message appears once per command.
-- Reports of functions with package state called during elaboration are printed once, when
-- they run: not again when the calls are replayed on fresh package state (#390).
package elab_report_replay_pkg is
  type counter_t is protected
    impure function add(n : integer) return integer;
  end protected;
  impure function f1 return integer;
  impure function f2 return integer;
end package;

package body elab_report_replay_pkg is
  type counter_t is protected body
    variable v : integer := 0;
    impure function add(n : integer) return integer is
    begin
      v := v + n;
      return v;
    end function;
  end protected body;

  shared variable counter : counter_t;

  impure function f1 return integer is
    constant n : integer := counter.add(1);
  begin
    report "f1 returns " & integer'image(n);
    return n;
  end function;

  impure function f2 return integer is
  begin
    return counter.add(10);
  end function;
end package body;

use work.elab_report_replay_pkg.all;

entity elab_report_replay is
end entity;

architecture a of elab_report_replay is
  constant c1 : integer := f1;
  constant c2 : integer := f2;
  constant c3 : integer := f1;
begin
  process
  begin
    report "c1 = " & integer'image(c1) & ", c2 = " & integer'image(c2) & ", c3 = " & integer'image(c3);
    wait;
  end process;
end architecture;
