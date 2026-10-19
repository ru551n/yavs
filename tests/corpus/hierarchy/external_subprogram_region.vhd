-- difftest: top=external_subprogram_region std=2008 ref=hand ignore=order
-- ignore=order: the processes of both iterations report in the same cycles; the LRM leaves
-- their order unspecified.
-- ref=hand: the reference simulator binds an external name in a subprogram of a generate
-- iteration once for all iterations (the second iteration reads and drives the first one's
-- leaf, "after await 10" twice); a relative path starts at the innermost concurrent region
-- of each elaborated copy (LRM 8.7), which another reference simulator follows: the expected
-- values are its output.
-- External names in subprograms of a block in a for-generate (LRM 8.7, #255): functions of
-- the block read their own iteration's instance through relative paths; procedures of a
-- process drive (with the process's driver) and wait on an external signal.
entity leaf is
  generic (id : integer := 0);
end;
architecture a of leaf is
  signal cnt : integer := 0;
  constant tag : integer := id * 10;
begin
  process (cnt)
  begin
    report "leaf" & integer'image(id) & " cnt=" & integer'image(cnt);
  end process;
end;

entity external_subprogram_region is end;
architecture t of external_subprogram_region is
  signal go : bit := '0';
begin
  g : for i in 1 to 2 generate
    u : entity work.leaf generic map (id => i);
    -- A function of the generate region reading its own instance through a relative path.
    b : block
      impure function tagof return integer is
      begin
        return <<constant ^.u.tag : integer>>;
      end function;
      impure function cntof return integer is
      begin
        return <<signal ^.u.cnt : integer>>;
      end function;
    begin
      process
        procedure bump is
        begin
          <<signal ^.u.cnt : integer>> <= cntof + tagof;
        end procedure;
        procedure await is
        begin
          wait on <<signal ^.u.cnt : integer>>;
        end procedure;
      begin
        wait until go = '1';
        bump;
        await;
        report "after await " & integer'image(cntof);
        bump;
        wait for 1 ns;
        assert cntof = 2 * tagof severity failure;
        wait;
      end process;
    end block;
  end generate;
  go <= '1' after 1 ns;
end;
