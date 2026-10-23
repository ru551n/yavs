-- A process without a wait statement; it ends the run in its first activation.
entity stuck_nowait is end entity;
architecture a of stuck_nowait is
begin
  p : process
  begin
    report "once";
    std.env.finish;
  end process;
end architecture;
