-- difftest: top=elab_block_function std=2008
-- A function declared in a block or an architecture reads a generic and a constant of the
-- enclosing regions and is called during elaboration (#494): in a constant that sizes a
-- signal, in a generic map actual of an instance and in a generate bound. A generic whose
-- default is a literal is read through the function too (#499); the generics of the instance
-- below get their values from calls, so none of them is folded.
entity elab_block_function_sub is
  generic (n : natural; sev : severity_level);
end entity;

architecture a of elab_block_function_sub is
begin
  process
  begin
    report "sub n=" & integer'image(n) & " sev=" & severity_level'image(sev);
    wait;
  end process;
end architecture;

entity elab_block_function_inner is
  generic (width : natural; wide : boolean);
end entity;

architecture t of elab_block_function_inner is
  constant base : natural := width + 2;
  function pick return severity_level is
  begin
    if wide then
      return error;
    end if;
    return warning;
  end function;
begin
  b : block
    function scaled(k : natural) return natural is
    begin
      return width * k + base;
    end function;
    constant total : natural := scaled(3);
    signal v : bit_vector(1 to total);
  begin
    process
    begin
      wait for total * 1 ns;
      report "block total=" & integer'image(total) & " len=" & integer'image(v'length);
      wait;
    end process;
    u : entity work.elab_block_function_sub
      generic map (n => scaled(1), sev => pick);
    g : for i in 1 to scaled(0) - base + 2 generate
      process
      begin
        report "gen " & integer'image(i);
        wait;
      end process;
    end generate;
  end block;
end architecture;

entity elab_block_function is
end entity;

architecture t of elab_block_function is
  -- The actuals are calls, so the inner generics have no folded value.
  function twice(n : natural) return natural is
  begin
    return 2 * n;
  end function;
  function yes(b : boolean) return boolean is
  begin
    return b;
  end function;
begin
  u0 : entity work.elab_block_function_inner generic map (width => twice(3), wide => yes(false));
  u1 : entity work.elab_block_function_inner generic map (width => twice(2), wide => yes(true));
end architecture;
