-- difftest: top=all_local_subprogram std=2008
-- process (all) is sensitive to the signals read by the procedures and functions declared in
-- the process, not to those of subprograms declared outside it and not to the signals passed
-- as parameters.
entity all_local_subprogram is end entity;

architecture a of all_local_subprogram is
  signal s1, s2, s3, s4 : integer := 0;
  procedure outer is
  begin
    report "outer reads s2=" & integer'image(s2);
  end procedure;
begin
  p : process (all)
    procedure inner is
    begin
      report "inner reads s1=" & integer'image(s1) & " s3=" & integer'image(s3);
    end procedure;
    procedure param (signal x : in integer) is
    begin
      report "param x=" & integer'image(x);
    end procedure;
  begin
    inner;
    outer;
    param(s4);
  end process;

  stim : process
  begin
    wait for 1 ns;
    s1 <= 1;
    wait for 1 ns;
    s2 <= 1;
    wait for 1 ns;
    s3 <= 1;
    wait for 1 ns;
    s4 <= 1;
    wait;
  end process;
end architecture;
