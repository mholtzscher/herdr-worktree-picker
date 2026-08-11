{ inputs, pkgs, ... }:

{
  packages = with pkgs; [
    gh
    git
    inputs.herdr.packages.${pkgs.system}.default
  ];

  languages.rust = {
    enable = true;
    channel = "stable";
  };
}
