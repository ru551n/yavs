-- Scoreboard as a protected type: a ring-buffer queue with push, pop and check.
entity bench_protected_scoreboard is
  generic (N : natural := 150000);
end entity;

architecture sim of bench_protected_scoreboard is
  type scoreboard_t is protected
    procedure push(v : integer);
    impure function pop return integer;
    impure function count return natural;
    procedure check(got : integer);
    impure function errors return natural;
  end protected;

  type scoreboard_t is protected body
    type buf_t is array (0 to 1023) of integer;
    variable buf  : buf_t;
    variable head : natural := 0;
    variable tail : natural := 0;
    variable n    : natural := 0;
    variable errs : natural := 0;

    procedure push(v : integer) is
    begin
      buf(tail) := v;
      tail := (tail + 1) mod 1024;
      n := n + 1;
    end procedure;

    impure function pop return integer is
      variable v : integer;
    begin
      v := buf(head);
      head := (head + 1) mod 1024;
      n := n - 1;
      return v;
    end function;

    impure function count return natural is
    begin
      return n;
    end function;

    procedure check(got : integer) is
    begin
      if pop /= got then
        errs := errs + 1;
      end if;
    end procedure;

    impure function errors return natural is
    begin
      return errs;
    end function;
  end protected body;

  shared variable sb : scoreboard_t;
begin
  process
  begin
    for i in 1 to N loop
      sb.push(i * 3);
      if sb.count > 64 then
        sb.check((i - 64) * 3);
      end if;
    end loop;
    assert sb.errors = 0 report "scoreboard mismatches" severity failure;
    report "protected_scoreboard left=" & integer'image(sb.count);
    wait;
  end process;
end architecture;
