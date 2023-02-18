#! /bin/bash

export appPath=./target/release/
export binaryName=rust_tutorial

function start() {
  # pid=$(pgrep $binaryName)
  pid=$(pidof $binaryName)
  # echo "$binaryName pid  = $pid"

  if [ "$pid" == "" ]; then
    cd $appPath
    ./$binaryName > nohup.out 2>&1 &
    echo ./$binaryName " start"
  else
    echo "$binaryName is running already"
  fi
}

function stop() {
  if [ -f "$appPath/nohup.out" ]; then
    mv "$appPath/nohup.out" $appPath/log_backup/nohup.out."$(date "+%Y%m%d-%H%M%S")"
  fi

  pid=$(pidof $binaryName)

  if [ "$pid" != "" ]; then
    kill "$pid"
    echo $binaryName " stop"
  else
    echo "pid of $binaryName is empty"
  fi
}

function update() {
  stop
  sleep 1
  build
  sleep 1
  start
}

function restart() {
  stop
  sleep 1
  start
}

function build() {
  cargo build --release
  echo "success building"
}

function help() {
  echo "$0 start|stop|restart|update"
}

if [ "$1" == "start" ]; then
  start
elif [ "$1" == "stop" ]; then
  stop
elif [ "$1" == "restart" ]; then
  restart
elif [ "$1" == "update" ]; then
  update
else
  help
fi
