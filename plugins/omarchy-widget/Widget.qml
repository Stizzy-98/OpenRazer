import QtQuick
import QtQuick.Controls
import Quickshell
import qs.Ui
import qs.Commons

// Quick keyboard-lighting effect switcher for Razer Control. Every action shells out to
// `razer-cli` (fire-and-forget, same trust model as running it from a terminal) rather than
// reimplementing the daemon's bincode socket protocol here - the CLI is already the stable,
// tested entry point. This is a fast preset-switcher, not a full settings editor: colors beyond
// the swatch set, sensitivity/decay, and per-key painting all stay in the razer-settings GTK app.
BarWidget {
  id: root
  moduleName: "razercontrol.lighting"

  readonly property var swatches: [
    { name: "Red", r: 255, g: 0, b: 0 },
    { name: "Orange", r: 255, g: 128, b: 0 },
    { name: "Yellow", r: 255, g: 255, b: 0 },
    { name: "Green", r: 0, g: 255, b: 0 },
    { name: "Cyan", r: 0, g: 255, b: 255 },
    { name: "Blue", r: 0, g: 0, b: 255 },
    { name: "Purple", r: 128, g: 0, b: 255 },
    { name: "Magenta", r: 255, g: 0, b: 255 },
    { name: "Pink", r: 255, g: 105, b: 180 },
    { name: "White", r: 255, g: 255, b: 255 },
    { name: "Warm White", r: 255, g: 214, b: 170 },
    { name: "Off", r: 0, g: 0, b: 0 }
  ]
  readonly property int swatchColumns: 4

  // "" = showing the main effect list. "static"/"reactive" = showing the color swatch grid for
  // that effect instead.
  property string pickingColorFor: ""
  // Keyboard/highlight cursor, shared by both the effect list and the swatch grid (re-clamped to
  // whichever's model size whenever the view switches).
  property int cursorIndex: 0

  function run(args) {
    // One-shot fire-and-forget spawn, not a reused stateful Process - reusing a single Process
    // and mutating .command/.running on it raced on rapid re-selection (the first tap after
    // switching effects silently reapplied whatever the previous command was; only the second
    // tap's command actually took effect).
    Quickshell.execDetached(["razer-cli"].concat(args))
  }

  function applyStatic(r, g, b) { run(["standard-effect", "static", String(r), String(g), String(b)]) }
  function applyReactive(r, g, b) { run(["standard-effect", "reactive", "2", String(r), String(g), String(b)]) }

  function pickColor(sw) {
    if (pickingColorFor === "reactive") applyReactive(sw.r, sw.g, sw.b)
    else applyStatic(sw.r, sw.g, sw.b)
    pickingColorFor = ""
    popup.open = false
  }

  readonly property var effectRows: [
    { label: "Off", action: function() { run(["standard-effect", "off"]) } },
    { label: "Static …", action: function() { pickingColorFor = "static"; cursorIndex = 0 } },
    { label: "Wave →", action: function() { run(["standard-effect", "wave", "1"]) } },
    { label: "Wave ←", action: function() { run(["standard-effect", "wave", "2"]) } },
    { label: "Breathing", action: function() { run(["standard-effect", "breathing", "1", "255", "255", "255", "0", "0", "0"]) } },
    { label: "Reactive …", action: function() { pickingColorFor = "reactive"; cursorIndex = 0 } },
    { label: "Spectrum", action: function() { run(["standard-effect", "spectrum"]) } },
    { label: "Starlight", action: function() { run(["standard-effect", "starlight", "1", "2", "255", "255", "255", "0", "0", "0"]) } },
    { label: "Wheel", action: function() { run(["wheel", "1", "50"]) } },
    { label: "Audio Meter", action: function() { run(["audio-meter", "1", "0", "255", "0", "100", "30", "100"]) } },
    { label: "Stars", action: function() { run(["stars", "2"]) } },
    { label: "Ripple", action: function() { run(["ripple", "1", "0", "0", "0", "3"]) } },
    { label: "CPU Temperature", action: function() { run(["temperature", "45", "90"]) } }
  ]

  function activateCursor() {
    if (pickingColorFor === "") {
      var row = effectRows[cursorIndex]
      if (row) {
        row.action()
        if (pickingColorFor === "") popup.open = false
      }
    } else {
      var sw = swatches[cursorIndex]
      if (sw) pickColor(sw)
    }
  }

  function moveCursor(dx, dy) {
    if (pickingColorFor === "") {
      var count = effectRows.length
      cursorIndex = ((cursorIndex + dy) % count + count) % count
    } else {
      var cols = swatchColumns
      var count2 = swatches.length
      var row = Math.floor(cursorIndex / cols)
      var col = cursorIndex % cols
      var rows = Math.ceil(count2 / cols)
      col = ((col + dx) % cols + cols) % cols
      row = ((row + dy) % rows + rows) % rows
      var next = row * cols + col
      cursorIndex = Math.min(next, count2 - 1)
    }
  }

  visible: true
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: ""
    tooltipText: "Razer Lighting"
    onPressed: {
      root.cursorIndex = 0
      popup.open = !popup.open
    }
  }

  PopupCard {
    id: popup
    anchorItem: button
    bar: root.bar
    contentWidth: Style.space(200)
    // Cap the popup itself at a sane max and let the ListView below scroll the rest.
    contentHeight: popup.fittedContentHeight(list.visible ? list.contentHeight : swatchGrid.height, Style.space(320))
    onOpenChanged: if (!open) root.pickingColorFor = ""

    PanelKeyCatcher {
      anchors.fill: parent
      onMoveRequested: function(dx, dy) { root.moveCursor(dx, dy) }
      onActivateRequested: root.activateCursor()
      onReturnRequested: root.activateCursor()
      onCloseRequested: popup.open = false

      ListView {
        id: list
        visible: root.pickingColorFor === ""
        anchors.fill: parent
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        model: root.effectRows
        currentIndex: root.cursorIndex
        onCurrentIndexChanged: if (currentIndex >= 0) positionViewAtIndex(currentIndex, ListView.Contain)
        highlightFollowsCurrentItem: true
        highlight: Rectangle {
          color: Color.menu.selectedBackground
          radius: Style.cornerRadius
        }

        delegate: Rectangle {
          required property var modelData
          required property int index
          width: list.width
          height: Style.spacing.popupRowHeight
          color: "transparent"

          Text {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: Style.spacing.sm
            text: modelData.label
            color: index === root.cursorIndex ? Color.menu.selectedText : Color.menu.text
            font.pixelSize: Style.font.body
            font.family: Style.font.menuFamily
          }

          HoverHandler { onHoveredChanged: if (hovered) root.cursorIndex = index }
          TapHandler {
            onTapped: {
              root.cursorIndex = index
              root.activateCursor()
            }
          }
        }
      }

      Column {
        id: swatchGrid
        visible: root.pickingColorFor !== ""
        width: parent.width
        spacing: Style.spacing.xxs

        Rectangle {
          width: parent.width
          height: Style.spacing.popupRowHeight
          radius: Style.cornerRadius
          color: backHover.hovered ? Color.menu.selectedBackground : "transparent"

          Text {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: Style.spacing.sm
            text: "← Back"
            color: backHover.hovered ? Color.menu.selectedText : Color.menu.text
            font.pixelSize: Style.font.body
            font.family: Style.font.menuFamily
          }

          HoverHandler { id: backHover }
          TapHandler { onTapped: root.pickingColorFor = "" }
        }

        Grid {
          width: parent.width
          columns: root.swatchColumns
          spacing: Style.spacing.sm
          leftPadding: Style.spacing.sm
          rightPadding: Style.spacing.sm
          bottomPadding: Style.spacing.sm

          Repeater {
            model: root.pickingColorFor !== "" ? root.swatches : []
            delegate: Rectangle {
              required property var modelData
              required property int index
              width: Style.space(32)
              height: Style.space(32)
              radius: width / 2
              color: Qt.rgba(modelData.r / 255, modelData.g / 255, modelData.b / 255, 1.0)
              border.width: (index === root.cursorIndex || swatchHover.hovered) ? 2 : 1
              border.color: (index === root.cursorIndex || swatchHover.hovered) ? Color.accent : Color.menu.border

              HoverHandler { id: swatchHover; onHoveredChanged: if (hovered) root.cursorIndex = index }
              TapHandler {
                onTapped: {
                  root.cursorIndex = index
                  root.activateCursor()
                }
              }
            }
          }
        }
      }
    }
  }
}
