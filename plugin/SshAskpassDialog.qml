import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons
import qs.Ui

Item {
  id: root

  signal finished(string result, string password)

  // "passphrase" (type and press Enter), "confirm" (Allow/Deny, for
  // ssh-add -c) or "notify" (a message OpenSSH dismisses by killing us).
  property string mode: "passphrase"
  property string prompt: ""
  property bool shown: false
  property bool closing: false
  property bool errorFlash: false
  property int shakeOffset: 0

  property string fontFamily: Style.font.menuFamily
  property color accent: Color.polkit.accent
  property color background: Color.polkit.background
  property color foreground: Color.polkit.text
  property color textError: Color.polkit.textError
  property var borderSpec: Border.surfaceSpec("polkit", errorFlash ? "border-error" : "border", errorFlash ? Color.polkit.borderError : Color.polkit.border, Math.max(1, Style.space(2)), "border-alpha")
  readonly property int contentMargin: Style.spacing.panelPadding
  readonly property int fieldHeight: Math.max(Style.space(42), Style.spacing.controlHeight)
  // OpenSSH's notifier for FIDO keys: "Confirm user presence for key …".
  readonly property bool touchMode: mode === "notify" && /user presence/i.test(prompt)
  readonly property int cardWidth: Math.min(Style.space(420), Math.max(Style.space(260), panel.width - Style.gapsOut * 2))

  function open(req) {
    closeTimer.stop()
    closing = false
    mode = String(req.mode || "passphrase")
    prompt = String(req.prompt || "")
    clearInputs()
    shown = true
    Qt.callLater(refocus)
  }

  function close() {
    clearInputs()
    if (!shown) return
    closing = true
    shown = false
    closeTimer.restart()
  }

  function clearInputs() {
    passwordField.input.text = ""
  }

  function respond(result) {
    if (!shown) return
    // Only an answered passphrase prompt hands the typed text over; a
    // cancel or a confirmation never touches it.
    var password = result === "ok" && mode === "passphrase" ? passwordField.input.text : ""
    close()
    finished(result, password)
  }

  function refocus() {
    if (!shown) return
    if (mode === "passphrase") passwordField.input.forceActiveFocus()
    else if (mode === "confirm") {
      if (!allowButton.activeFocus) denyButton.forceActiveFocus()
    } else keyCatcher.forceActiveFocus()
  }

  function triggerFailureFeedback() {
    errorFlash = true
    errorTimer.restart()
    shakeAnimation.restart()
    Qt.callLater(refocus)
  }

  Timer {
    id: closeTimer
    interval: 300
    repeat: false
    onTriggered: root.closing = false
  }

  Timer {
    id: errorTimer
    interval: 1200
    repeat: false
    onTriggered: root.errorFlash = false
  }

  SequentialAnimation {
    id: shakeAnimation
    NumberAnimation { target: root; property: "shakeOffset"; to: -8; duration: 35; easing.type: Easing.OutQuad }
    NumberAnimation { target: root; property: "shakeOffset"; to: 8; duration: 50; easing.type: Easing.InOutQuad }
    NumberAnimation { target: root; property: "shakeOffset"; to: 0; duration: 55; easing.type: Easing.OutQuad }
  }

  component PasswordField: Item {
    id: field
    property alias input: textInput
    property string placeholder: ""
    signal accepted()

    width: parent.width
    height: root.fieldHeight

    Row {
      anchors.fill: parent
      spacing: Style.space(14)

      Text {
        text: ""
        color: root.errorFlash ? root.textError : root.accent
        font.family: root.fontFamily
        font.pixelSize: Style.font.iconLarge
        width: Style.space(26)
        height: parent.height
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
      }

      Item {
        width: parent.width - Style.space(40)
        height: parent.height

        TextInput {
          id: textInput
          anchors.fill: parent
          verticalAlignment: TextInput.AlignVCenter
          activeFocusOnPress: true
          clip: true
          selectionColor: Util.alpha(root.accent, 0.45)
          selectedTextColor: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.iconLarge
          echoMode: TextInput.Password
          passwordCharacter: "•"
          inputMethodHints: Qt.ImhSensitiveData | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
          color: root.errorFlash ? root.textError : root.foreground
          cursorVisible: activeFocus
          enabled: root.shown
          onAccepted: field.accepted()
          Keys.onPressed: function(event) {
            if (event.key === Qt.Key_Escape) {
              root.respond("cancel")
              event.accepted = true
            }
          }
        }

        Text {
          textFormat: Text.PlainText
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          text: field.placeholder
          color: root.foreground
          opacity: 0.36
          font.family: root.fontFamily
          font.pixelSize: Style.font.iconLarge
          elide: Text.ElideRight
          visible: textInput.text.length === 0
        }

        Rectangle {
          width: Math.max(1, Style.space(2))
          height: Style.space(24)
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
          color: root.foreground
          visible: textInput.activeFocus && textInput.text.length === 0
        }

        MouseArea {
          anchors.fill: parent
          onClicked: textInput.forceActiveFocus()
        }
      }
    }
  }

  PanelWindow {
    id: panel
    visible: root.shown || root.closing
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    WlrLayershell.namespace: "omarchy-ssh-askpass"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: root.shown ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None
    exclusionMode: ExclusionMode.Ignore

    Rectangle {
      anchors.fill: parent
      color: Color.polkit.scrim
    }

    MouseArea {
      anchors.fill: parent
      onClicked: root.refocus()
    }

    BorderSurface {
      id: card
      width: root.cardWidth
      height: content.implicitHeight + card.contentTopInset + card.contentBottomInset
      radius: Style.cornerRadius
      anchors.centerIn: parent
      anchors.horizontalCenterOffset: root.shakeOffset
      color: root.background
      borderSpec: root.borderSpec
      padding: root.contentMargin

      MouseArea { anchors.fill: parent; onClicked: root.refocus() }

      Item {
        id: keyCatcher
        anchors.fill: parent
        focus: true

        Keys.priority: Keys.BeforeItem
        Keys.onPressed: function(event) {
          if (event.key === Qt.Key_Escape) {
            root.respond("cancel")
            event.accepted = true
          } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            // Only the passphrase field submits on Enter; a confirmation
            // takes an explicit Allow.
            if (root.mode === "passphrase") root.respond("ok")
            event.accepted = true
          }
        }
      }

      Column {
        id: content
        anchors.fill: parent
        anchors.topMargin: card.contentTopInset
        anchors.rightMargin: card.contentRightInset
        anchors.bottomMargin: card.contentBottomInset
        anchors.leftMargin: card.contentLeftInset
        spacing: Style.space(10)

        // Escape from a focused button (the key catcher only sees keys
        // while it has focus itself).
        Keys.onEscapePressed: function(event) {
          root.respond("cancel")
          event.accepted = true
        }

        Row {
          visible: root.touchMode
          width: parent.width
          height: root.fieldHeight
          spacing: Style.space(14)

          OpticalGlyph {
            id: touchGlyph
            width: Style.space(26)
            height: parent.height
            text: "\udb80\udf06"
            fontFamily: root.fontFamily
            fontSize: Style.font.iconLarge
            color: root.accent

            SequentialAnimation on opacity {
              running: root.touchMode && root.shown
              loops: Animation.Infinite
              onStopped: touchGlyph.opacity = 1
              NumberAnimation { to: 0.35; duration: 700; easing.type: Easing.InOutSine }
              NumberAnimation { to: 1; duration: 700; easing.type: Easing.InOutSine }
            }
          }

          Text {
            width: parent.width - Style.space(40)
            height: parent.height
            textFormat: Text.PlainText
            text: "Touch your security key"
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.iconLarge
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
          }
        }

        Text {
          width: parent.width
          visible: text.length > 0
          textFormat: Text.PlainText
          text: root.prompt
          wrapMode: Text.Wrap
          color: root.foreground
          opacity: root.touchMode ? 0.7 : 1
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        PasswordField {
          id: passwordField
          visible: root.mode === "passphrase"
          placeholder: /\bPIN\b/.test(root.prompt) ? "PIN" : "Passphrase"
          onAccepted: root.respond("ok")
        }

        Row {
          visible: root.mode !== "passphrase" && !root.touchMode
          anchors.right: parent.right
          spacing: Style.spacing.controlGap

          Button {
            id: denyButton
            visible: root.mode === "confirm"
            text: "Deny"
            bordered: true
            focusable: true
            onClicked: root.respond("cancel")
          }
          Button {
            id: allowButton
            visible: root.mode === "confirm"
            text: "Allow"
            bordered: true
            focusable: true
            selected: true
            onClicked: root.respond("ok")
          }
          Button {
            visible: root.mode === "notify"
            text: "Dismiss"
            bordered: true
            focusable: true
            selected: true
            onClicked: root.respond("cancel")
          }
        }
      }
    }

    Rectangle {
      width: Math.min(titleText.implicitWidth + Style.space(24), panel.width - Style.gapsOut * 2)
      height: Style.space(28)
      anchors.horizontalCenter: card.horizontalCenter
      anchors.bottom: card.top
      anchors.bottomMargin: Style.space(10)
      radius: Style.cornerRadius
      color: root.background

      Text {
        id: titleText
        textFormat: Text.PlainText
        anchors.fill: parent
        anchors.leftMargin: Style.space(12)
        anchors.rightMargin: Style.space(12)
        text: "OpenSSH"
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideMiddle
      }
    }
  }
}
